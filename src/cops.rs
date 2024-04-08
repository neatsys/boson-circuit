use std::{
    cmp::Ordering::{Equal, Greater},
    collections::BTreeMap,
};

use serde::{Deserialize, Serialize};

use crate::{
    event::{erased::OnEvent, Timer},
    net::{events::Recv, Addr, All, SendMessage},
    util::Payload,
};

// "key" under COPS context, "id" under Boson's logical clock context
pub type KeyId = u32;

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct Put<V, A> {
    key: KeyId,
    value: Payload,
    deps: BTreeMap<KeyId, V>,
    client_addr: A,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct PutOk<V> {
    version: V,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct Get<A> {
    key: KeyId,
    client_addr: A,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct GetOk<V, A> {
    put: Put<V, A>,
    version: V,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct SyncKey<V, A> {
    put: Put<V, A>,
    version: V,
}

pub trait ClientNet<A, V>: SendMessage<A, GetOk<V, A>> + SendMessage<A, PutOk<V>> {}
impl<T: SendMessage<A, GetOk<V, A>> + SendMessage<A, PutOk<V>>, A, V> ClientNet<A, V> for T {}

pub trait ServerNet<A, V>:
    SendMessage<u8, Put<V, A>> + SendMessage<u8, Get<A>> + SendMessage<All, SyncKey<V, A>>
{
}
impl<
        T: SendMessage<u8, Put<V, A>> + SendMessage<u8, Get<A>> + SendMessage<All, SyncKey<V, A>>,
        A,
        V,
    > ServerNet<A, V> for T
{
}

pub trait VersionService {
    type Version;
    fn merge_and_increment_once(
        &self,
        id: KeyId,
        previous: Option<Self::Version>,
        deps: Vec<Self::Version>,
    ) -> anyhow::Result<()>;
}

pub trait Version: PartialOrd + Clone + Send + Sync + 'static {}
impl<T: PartialOrd + Clone + Send + Sync + 'static> Version for T {}

pub struct VersionOk<V, A>(pub Put<V, A>, pub V);

// pub struct Client<V> {
// }

pub struct Server<N, CN, VS, V, A> {
    store: BTreeMap<KeyId, (Put<V, A>, V)>,
    net: N,
    client_net: CN,
    #[allow(unused)]
    version_worker: VS,
}

impl<N, CN: ClientNet<A, V>, A: Addr, V: Version, VS> OnEvent<Recv<Get<A>>>
    for Server<N, CN, VS, V, A>
{
    fn on_event(&mut self, Recv(get): Recv<Get<A>>, _: &mut impl Timer) -> anyhow::Result<()> {
        if let Some((put, version)) = self.store.get(&get.key) {
            let get_ok = GetOk {
                put: put.clone(),
                version: version.clone(),
            };
            return self.client_net.send(get.client_addr, get_ok);
        }
        Ok(())
    }
}

impl<N: ServerNet<A, V>, CN: ClientNet<A, V>, A, V: Version, VS: VersionService<Version = V>>
    OnEvent<Recv<Put<V, A>>> for Server<N, CN, VS, V, A>
{
    fn on_event(&mut self, Recv(put): Recv<Put<V, A>>, _: &mut impl Timer) -> anyhow::Result<()> {
        if let Some((_, version)) = self.store.get(&put.key) {
            if put
                .deps
                .iter()
                .all(|(_, v)| matches!(version.partial_cmp(v), Some(Greater | Equal)))
            {
                let put_ok = PutOk {
                    version: version.clone(),
                };
                return self.client_net.send(put.client_addr, put_ok);
            }
        }
        Ok(())
    }
}

impl<N, CN: ClientNet<A, V>, A: Addr, V: Version, VS: VersionService<Version = V>>
    OnEvent<Recv<SyncKey<V, A>>> for Server<N, CN, VS, V, A>
{
    fn on_event(
        &mut self,
        Recv(sync): Recv<SyncKey<V, A>>,
        _: &mut impl Timer,
    ) -> anyhow::Result<()> {
        // TODO
        self.store.insert(sync.put.key, (sync.put, sync.version));
        Ok(())
    }
}

impl<
        N: ServerNet<A, V>,
        CN: ClientNet<A, V>,
        A,
        V: PartialOrd + Clone,
        VS: VersionService<Version = V>,
    > OnEvent<VersionOk<V, A>> for Server<N, CN, VS, V, A>
{
    fn on_event(
        &mut self,
        VersionOk(put, version): VersionOk<V, A>,
        _: &mut impl Timer,
    ) -> anyhow::Result<()> {
        let sync_key = SyncKey { put, version };
        self.net.send(All, sync_key)
        // TODO
    }
}
