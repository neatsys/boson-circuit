use std::{fmt::Debug, io::ErrorKind, net::SocketAddr, sync::Arc, time::Duration};

use lru::LruCache;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::Instant,
};
use tracing::warn;

use crate::event::{erased::OnEvent, OnTimer, SendEvent, Timer};

use super::{Buf, IterAddr, SendMessage};

#[derive(Debug, Clone)]
pub struct Udp(pub Arc<tokio::net::UdpSocket>);

impl Udp {
    pub async fn recv_session(
        &self,
        mut on_buf: impl FnMut(&[u8]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let mut buf = vec![0; 1 << 16];
        loop {
            let (len, _) = self.0.recv_from(&mut buf).await?;
            on_buf(&buf[..len])?
        }
    }
}

impl<B: Buf> SendMessage<SocketAddr, B> for Udp {
    fn send(&mut self, dest: SocketAddr, buf: B) -> anyhow::Result<()> {
        let socket = self.0.clone();
        // a broken error propagation here. no observation to the failure of `send_to`
        // by definition `SendMessage` is one-way (i.e. no complete notification) unreliable net
        // interface, so this is fine, just kindly note the fact
        // alternatively, collect sending tasks into a `JoinSet`
        // however that cannot be owned by `impl OnEvent`, which does not have a chance to poll
        // so not an ideal alternation and not conducted for now
        tokio::spawn(async move { socket.send_to(buf.as_ref(), dest).await.unwrap() });
        Ok(())
    }
}

impl<B: Buf> SendMessage<IterAddr<'_, SocketAddr>, B> for Udp {
    fn send(&mut self, dest: IterAddr<'_, SocketAddr>, buf: B) -> anyhow::Result<()> {
        for addr in dest.0 {
            self.send(addr, buf.clone())?
        }
        Ok(())
    }
}

const TCP_MAX_CONNECTION_NUM: usize = 1024;

const TCP_MAX_BUF_LEN: usize = 1 << 20;

const TCP_PREAMBLE_LEN: usize = 32;

// a construction that enables connection reusing and thottling
// the client side of a connection informs its server address to the connected
// server with preamble, so if later a message need to be delivered in the
// opposite direction, it can go through the existing connection
// TODO consider generalize this connection over underlying transportation
// protocols to be reused e.g. for QUIC
#[derive(Debug)]
pub struct TcpControl<B, F> {
    // cached connections based on the last *outgoing* traffic
    // the incoming messages does not prompt a connection in this cache. if an incoming connection
    // is not being reused for egressing for a while, it may get evicted from this cache even if the
    // connection is still actively receiving messages
    // this does not affect the incoming traffic. even if the connection is evicted, only the
    // `write_task` exits (due to the dropped egress sender). `read_task` will still be alive and
    // forward incoming messages by calling `on_buf`
    // if afterward there's outgoing messages to the remote address, a new connection will be
    // created and pushed into the cache, the connection will be accepted by remote as an incoming
    // stream, which will be unconditionally `put` into the cache, replace the previous egress
    // sender and cause further outgoing traffic (on remote side, incoming traffic on local side)
    // migrate to the new connection. as the result, (eventually) there's at most one connection
    // between each pair of addresses
    // even if i find a way to easily promote connections on incoming messages, this strategy is
    // still necessary to eliminate duplicated connections between same pair of addresses
    connections: LruCache<SocketAddr, Connection<B>>,
    on_buf: F,
    preamble: bytes::Bytes,
}

#[derive(Debug)]
struct Connection<B> {
    sender: UnboundedSender<B>,
    used_at: Instant,
}

impl<B, F> TcpControl<B, F> {
    pub fn new(on_buf: F, addr: impl Into<Option<SocketAddr>>) -> Self {
        let addr = addr.into().unwrap_or(SocketAddr::from(([0, 0, 0, 0], 0)));
        let mut preamble = addr.to_string();
        assert!(preamble.len() < TCP_PREAMBLE_LEN);
        preamble += &vec![" "; TCP_PREAMBLE_LEN - preamble.len()].concat();
        Self {
            connections: LruCache::new(TCP_MAX_CONNECTION_NUM.try_into().unwrap()),
            on_buf,
            preamble: preamble.into_bytes().into(),
        }
    }
}

impl<B, F: FnMut(&[u8]) -> anyhow::Result<()>> TcpControl<B, F> {
    async fn read_task(mut stream: OwnedReadHalf, mut on_buf: F, remote: SocketAddr) {
        loop {
            let len = match stream.read_u64().await {
                Ok(len) => len as _,
                Err(err) => {
                    if !matches!(err.kind(), ErrorKind::UnexpectedEof) {
                        warn!("<<< {remote} {err}")
                    }
                    break;
                }
            };
            if let Err(err) = async {
                if len > TCP_MAX_BUF_LEN {
                    anyhow::bail!("invalid buffer length {len}")
                }
                let mut buf = vec![0; len];
                stream.read_exact(&mut buf).await?;
                on_buf(&buf)?;
                Ok(())
            }
            .await
            {
                warn!("<<< {remote} {err}");
                break;
            }
        }
    }
}

impl<B: Buf, F> TcpControl<B, F> {
    async fn write_task(
        mut stream: OwnedWriteHalf,
        mut receiver: UnboundedReceiver<B>,
        remote: SocketAddr,
    ) {
        while let Some(buf) = receiver.recv().await {
            if let Err(err) = async {
                stream.write_u64(buf.as_ref().len() as _).await?;
                stream.write_all(buf.as_ref()).await?;
                stream.flush().await
            }
            .await
            {
                warn!(">>> {remote} {err}");
                break;
            }
        }
    }
}

pub struct Outgoing<B>(SocketAddr, B);

pub struct Incoming(SocketAddr, TcpStream);

impl<B: Buf, F: FnMut(&[u8]) -> anyhow::Result<()> + Clone + Send + 'static> OnEvent<Outgoing<B>>
    for TcpControl<B, F>
{
    fn on_event(
        &mut self,
        Outgoing(remote, mut buf): Outgoing<B>,
        _: &mut impl Timer,
    ) -> anyhow::Result<()> {
        if let Some(connection) = self.connections.get_mut(&remote) {
            match connection.sender.send(buf) {
                Ok(()) => {
                    connection.used_at = Instant::now();
                    return Ok(());
                }
                Err(err) => {
                    self.connections.pop(&remote);
                    buf = err.0
                }
            }
        }
        while self.connections.len() >= TCP_MAX_CONNECTION_NUM {
            if self
                .connections
                .peek_lru()
                .as_ref()
                .unwrap()
                .1
                .used_at
                .elapsed()
                < Duration::from_secs(15)
            {
                warn!("explicit drop egress message due to reaching maximum concurrent connection number");
                return Ok(());
            }
            self.connections.pop_lru();
        }
        let (sender, receiver) = unbounded_channel::<B>();
        let preamble = self.preamble.clone();
        let on_buf = self.on_buf.clone();
        tokio::spawn(async move {
            let task = async {
                let mut stream = TcpStream::connect(remote).await?;
                stream.set_nodelay(true)?;
                stream.write_all(&preamble).await?;
                anyhow::Result::<_>::Ok(stream)
            };
            let stream = match task.await {
                Ok(stream) => stream,
                Err(err) => {
                    warn!(">>> {remote} {err}");
                    return;
                }
            };
            let (read, write) = stream.into_split();
            tokio::spawn(Self::read_task(read, on_buf, remote));
            tokio::spawn(Self::write_task(write, receiver, remote));
        });
        if sender.send(buf).is_err() {
            warn!(">>> {remote} new connection immediately fail")
        } else {
            self.connections.push(
                remote,
                Connection {
                    sender,
                    used_at: Instant::now(),
                },
            );
        }
        Ok(())
    }
}

impl<B: Buf, F: FnMut(&[u8]) -> anyhow::Result<()> + Clone + Send + 'static> OnEvent<Incoming>
    for TcpControl<B, F>
{
    fn on_event(
        &mut self,
        Incoming(remote, stream): Incoming,
        _: &mut impl Timer,
    ) -> anyhow::Result<()> {
        let (sender, receiver) = unbounded_channel::<B>();
        let (read, write) = stream.into_split();
        tokio::spawn(Self::read_task(read, self.on_buf.clone(), remote));
        tokio::spawn(Self::write_task(write, receiver, remote));
        if remote != SocketAddr::from(([0, 0, 0, 0], 0)) {
            let replaced = self.connections.put(
                remote,
                Connection {
                    sender,
                    used_at: Instant::now(),
                },
            );
            if replaced.is_some() {
                warn!("<<< {remote} replacing previous connection")
            }
        }
        Ok(())
    }
}

impl<B, F> OnTimer for TcpControl<B, F> {
    fn on_timer(&mut self, _: crate::event::TimerId, _: &mut impl Timer) -> anyhow::Result<()> {
        unreachable!()
    }
}

#[derive(Clone)]
pub struct Tcp<E>(pub E);

impl<E: SendEvent<Outgoing<B>>, B> SendMessage<SocketAddr, B> for Tcp<E> {
    fn send(&mut self, dest: SocketAddr, message: B) -> anyhow::Result<()> {
        self.0.send(Outgoing(dest, message))
    }
}

impl<E: SendEvent<Outgoing<B>>, B: Buf> SendMessage<IterAddr<'_, SocketAddr>, B> for Tcp<E> {
    fn send(&mut self, dest: IterAddr<'_, SocketAddr>, message: B) -> anyhow::Result<()> {
        for addr in dest.0 {
            SendMessage::send(self, addr, message.clone())?
        }
        Ok(())
    }
}

pub mod simplex {
    use std::net::SocketAddr;

    use tracing::warn;

    use crate::{
        event::{erased::Inline, UnreachableTimer},
        net::{Buf, IterAddr, SendMessage},
    };

    #[allow(clippy::type_complexity)]
    pub struct Tcp<B>(super::TcpControl<B, fn(&[u8]) -> anyhow::Result<()>>);

    impl<B> Default for Tcp<B> {
        fn default() -> Self {
            Self(super::TcpControl::new(
                |_| {
                    warn!("ignore ingress message of simplex connection");
                    Ok(())
                },
                None,
            ))
        }
    }

    impl<B: Buf> SendMessage<SocketAddr, B> for Tcp<B> {
        fn send(&mut self, dest: SocketAddr, message: B) -> anyhow::Result<()> {
            SendMessage::send(
                &mut super::Tcp(Inline(&mut self.0, &mut UnreachableTimer)),
                dest,
                message,
            )
        }
    }

    impl<B: Buf> SendMessage<IterAddr<'_, SocketAddr>, B> for Tcp<B> {
        fn send(&mut self, dest: IterAddr<'_, SocketAddr>, message: B) -> anyhow::Result<()> {
            SendMessage::send(
                &mut super::Tcp(Inline(&mut self.0, &mut UnreachableTimer)),
                dest,
                message,
            )
        }
    }
}

pub async fn tcp_accept_session(
    listener: TcpListener,
    mut sender: impl SendEvent<Incoming>,
) -> anyhow::Result<()> {
    loop {
        let (mut stream, peer_addr) = listener.accept().await?;
        let task = async {
            stream.set_nodelay(true)?;
            let mut preamble = vec![0; TCP_PREAMBLE_LEN];
            stream.read_exact(&mut preamble).await?;
            anyhow::Result::<_>::Ok(std::str::from_utf8(&preamble)?.trim_end().parse()?)
        };
        let remote = match task.await {
            Ok(remote) => remote,
            Err(err) => {
                warn!("{peer_addr} {err}");
                continue;
            }
        };
        // println!("{peer_addr} -> {remote}");
        sender.send(Incoming(remote, stream))?
    }
}
