use std::net::SocketAddr;

use augustus::{
    event::{
        self,
        erased::{session::Sender, Blanket, Session, Unify},
        Once, SendEvent,
    },
    lamport_mutex::{self, event::RequestOk, Causal, Lamport, LamportClock, Processor},
    net::{
        dispatch,
        session::{tcp, Tcp},
        Detach, Dispatch, IndexNet,
    },
};
use tokio::{
    net::TcpListener,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};

pub enum Event {
    Request,
    Release,
}

pub async fn untrusted_session(
    mut events: UnboundedReceiver<Event>,
    upcall: UnboundedSender<RequestOk>,
) -> anyhow::Result<()> {
    let addrs = (0..10)
        .map(|i| SocketAddr::from(([127, 0, 0, 1], 4000 + i)))
        .collect::<Vec<_>>();
    let id = 0u8;
    let addr = addrs[id as usize];

    let tcp_listener = TcpListener::bind(addr).await?;
    let mut dispatch_session = event::Session::new();
    let mut processor_session = Session::new();
    let mut causal_net_session = Session::new();

    let mut dispatch = event::Unify(event::Buffered::from(Dispatch::new(
        Tcp::new(addr)?,
        {
            let mut sender = Sender::from(causal_net_session.sender());
            move |buf: &_| lamport_mutex::on_buf(buf, &mut sender)
        },
        Once(dispatch_session.sender()),
    )?));
    let mut processor = Blanket(Unify(Processor::new(
        id,
        Detach(Sender::from(causal_net_session.sender())),
        upcall,
    )));
    let mut causal_net = Blanket(Unify(Causal::new(
        (0, id),
        Box::new(Sender::from(processor_session.sender()))
            as Box<dyn lamport_mutex::SendRecvEvent<LamportClock> + Send + Sync>,
        Box::new(Lamport(Sender::from(causal_net_session.sender()), id))
            as Box<dyn SendEvent<lamport_mutex::Update<LamportClock>> + Send + Sync>,
        lamport_mutex::MessageNet::<_, LamportClock>::new(IndexNet::new(
            dispatch::Net::from(dispatch_session.sender()),
            addrs,
            None,
        )),
    )?));

    let event_session = {
        let mut sender = Sender::from(processor_session.sender());
        async move {
            while let Some(event) = events.recv().await {
                match event {
                    Event::Request => sender.send(lamport_mutex::event::Request)?,
                    Event::Release => sender.send(lamport_mutex::event::Release)?,
                }
            }
            anyhow::Ok(())
        }
    };
    let tcp_accept_session = tcp::accept_session(tcp_listener, dispatch_session.sender());
    let dispatch_session = dispatch_session.run(&mut dispatch);
    let processor_session = processor_session.run(&mut processor);
    let causal_net_session = causal_net_session.run(&mut causal_net);

    tokio::select! {
        result = event_session => result?,
        result = tcp_accept_session => result?,
        result = dispatch_session => result?,
        result = processor_session => result?,
        result = causal_net_session => result?,
    }
    anyhow::bail!("unreachable")
}

// cSpell:words lamport upcall