use std::time::Duration;

use async_trait::async_trait;
use flume::{unbounded, Receiver, Sender};
use tokio_rusqlite::Connection;
use veilid_core::{
    CryptoKey, CryptoKind, CryptoTyped, DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, KeyPair,
    OperationId, RouteId, Target, TypedKey, ValueData, ValueSubkey, VeilidAPIResult, VeilidUpdate,
    CRYPTO_KIND_VLD0,
};

use crate::{
    other_err, store::Store, Node, Result, DDCP, DHT_SUBKEY_COUNT, SUBKEY_DB_VERSION,
    SUBKEY_SITE_ID,
};

pub async fn setup() -> (DDCP, Connection, MockStore) {
    let conn = DDCP::new_connection(None, "target/debug/crsqlite")
        .await
        .expect("db connection");
    let store = MockStore::new();
    let node = Box::new(MockNode::new(store.clone()).await);
    let ddcp = DDCP::new_conn_node(conn.clone(), node);
    (ddcp, conn, store)
}

pub struct MockNode {
    receiver: Receiver<VeilidUpdate>,
    sender: Sender<VeilidUpdate>,
    store: MockStore,

    keypair: KeyPair,

    site_id: Option<Vec<u8>>,
    db_version: Option<i64>,
}

impl MockNode {
    async fn new(store: MockStore) -> MockNode {
        let (node_sender, updates): (
            Sender<veilid_core::VeilidUpdate>,
            Receiver<veilid_core::VeilidUpdate>,
        ) = unbounded();

        MockNode {
            receiver: updates,
            sender: node_sender,
            store,
            keypair: KeyPair::default(),
            site_id: None,
            db_version: None,
        }
    }
}

#[async_trait]
impl Node for MockNode {
    fn clone_box(&self) -> Box<dyn Node> {
        Box::new(MockNode {
            receiver: self.receiver.clone(),
            sender: self.sender.clone(),
            store: self.store.clone(),
            keypair: self.keypair.clone(),
            site_id: self.site_id.clone(),
            db_version: self.db_version,
        })
    }

    fn store(&self) -> &dyn Store {
        &self.store
    }

    async fn wait_for_network(&self) -> Result<()> {
        Ok(())
    }

    async fn recv_updates_async(&self) -> Result<VeilidUpdate> {
        self.receiver.recv_async().await.map_err(other_err)
    }

    async fn shutdown(&self, _: Option<DHTRecordDescriptor>) -> VeilidAPIResult<()> {
        Ok(())
    }

    async fn app_call(&self, _target: Target, _message: Vec<u8>) -> VeilidAPIResult<Vec<u8>> {
        todo!("app_call");
    }

    async fn app_call_reply(
        &self,
        _call_id: OperationId,
        _message: Vec<u8>,
    ) -> VeilidAPIResult<()> {
        todo!("app_call_reply");
    }

    fn import_remote_private_route(&self, _blob: Vec<u8>) -> VeilidAPIResult<RouteId> {
        Ok(RouteId::default())
    }

    async fn new_private_route(&self) -> VeilidAPIResult<(RouteId, Vec<u8>)> {
        Ok((RouteId::default(), vec![]))
    }

    fn release_private_route(&self, _route_id: RouteId) -> VeilidAPIResult<()> {
        Ok(())
    }

    async fn create_dht_record(
        &self,
        _schema: DHTSchema,
        _kind: Option<CryptoKind>,
    ) -> VeilidAPIResult<DHTRecordDescriptor> {
        Ok(DHTRecordDescriptor::new(
            CryptoTyped::new(CRYPTO_KIND_VLD0, self.keypair.key),
            self.keypair.key,
            Some(self.keypair.secret),
            DHTSchema::DFLT(DHTSchemaDFLT {
                o_cnt: DHT_SUBKEY_COUNT,
            }),
        ))
    }

    async fn open_dht_record(
        &self,
        _key: TypedKey,
        _writer: Option<KeyPair>,
    ) -> VeilidAPIResult<DHTRecordDescriptor> {
        Ok(DHTRecordDescriptor::new(
            CryptoTyped::new(CRYPTO_KIND_VLD0, self.keypair.key),
            self.keypair.key,
            Some(self.keypair.secret),
            DHTSchema::DFLT(DHTSchemaDFLT {
                o_cnt: DHT_SUBKEY_COUNT,
            }),
        ))
    }

    async fn close_dht_record(&self, _key: TypedKey) -> VeilidAPIResult<()> {
        Ok(())
    }

    async fn delete_dht_record(&self, _key: TypedKey) -> VeilidAPIResult<()> {
        Ok(())
    }

    async fn get_dht_value(
        &self,
        _key: TypedKey,
        subkey: ValueSubkey,
        _force_refresh: bool,
    ) -> VeilidAPIResult<Option<ValueData>> {
        Ok(match subkey {
            SUBKEY_SITE_ID => match &self.site_id {
                Some(value) => {
                    Some(ValueData::new(value.to_owned(), self.keypair.key).expect("size ok"))
                }
                None => None,
            },
            SUBKEY_DB_VERSION => match self.db_version {
                Some(version) => Some(
                    ValueData::new(i64::to_be_bytes(version).to_vec(), self.keypair.key)
                        .expect("size ok"),
                ),
                None => None,
            },
            _ => None,
        })
    }

    async fn set_dht_value(
        &mut self,
        _key: TypedKey,
        subkey: ValueSubkey,
        data: Vec<u8>,
    ) -> VeilidAPIResult<Option<ValueData>> {
        Ok(match subkey {
            SUBKEY_SITE_ID => {
                self.site_id = Some(data.clone());
                Some(ValueData::new(data, self.keypair.key).expect("size ok"))
            }
            SUBKEY_DB_VERSION => {
                let dbv_arr: [u8; 8] = data.clone().try_into().expect("i64 sized data");
                self.db_version = Some(i64::from_be_bytes(dbv_arr));
                Some(ValueData::new(data, self.keypair.key).expect("size ok"))
            }
            _ => None,
        })
    }
}

#[derive(Clone)]
pub struct MockStore {
    local_sender: Sender<(TypedKey, KeyPair)>,
    local_receiver: Receiver<(TypedKey, KeyPair)>,
    remote_sender: Sender<(TypedKey, Vec<u8>)>,
    remote_receiver: Receiver<(TypedKey, Vec<u8>)>,
}

impl MockStore {
    fn new() -> MockStore {
        let (local_sender, local_receiver) = unbounded();
        let (remote_sender, remote_receiver) = unbounded();
        MockStore {
            local_sender,
            local_receiver,
            remote_sender,
            remote_receiver,
        }
    }
}

#[async_trait]
impl Store for MockStore {
    fn clone_box(&self) -> Box<dyn Store> {
        Box::new(MockStore {
            local_sender: self.local_sender.clone(),
            local_receiver: self.local_receiver.clone(),
            remote_sender: self.remote_sender.clone(),
            remote_receiver: self.remote_receiver.clone(),
        })
    }
    async fn upgrade(&self) -> VeilidAPIResult<()> {
        Ok(())
    }
    async fn remove_remote(&self, _name: String) -> VeilidAPIResult<()> {
        todo!("remove remote");
    }
    async fn remotes(&self) -> VeilidAPIResult<Vec<(String, CryptoTyped<CryptoKey>)>> {
        todo!("remotes");
    }
    async fn load_local_keypair(
        &self,
        _name: &str,
    ) -> VeilidAPIResult<Option<(TypedKey, KeyPair)>> {
        Ok(
            match self.local_receiver.recv_timeout(Duration::from_nanos(1)) {
                Ok(v) => Some(v),
                Err(_) => None,
            },
        )
    }
    async fn load_remote(
        &self,
        _name: &str,
    ) -> VeilidAPIResult<(Option<TypedKey>, Option<Vec<u8>>)> {
        Ok(
            match self.remote_receiver.recv_timeout(Duration::from_nanos(1)) {
                Ok((k, v)) => (Some(k), Some(v)),
                Err(_) => (None, None),
            },
        )
    }
    async fn store_local_keypair(
        &self,
        _name: &str,
        key: &TypedKey,
        owner: KeyPair,
    ) -> VeilidAPIResult<()> {
        self.local_sender
            .send((key.to_owned(), owner))
            .expect("send");
        Ok(())
    }
    async fn store_remote_key(&self, _name: &str, _key: &TypedKey) -> VeilidAPIResult<()> {
        todo!("store_remote_key");
    }
    async fn store_remote_site_id(&self, _name: &str, _site_id: Vec<u8>) -> VeilidAPIResult<()> {
        todo!("store_remote_site_id");
    }
}
