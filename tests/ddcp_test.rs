use std::sync::Arc;

use async_trait::async_trait;
use ddcp::{DDCP, Result, cli::path_as_string, Node, other_err};
use flume::{Sender, Receiver, unbounded};
use tokio_rusqlite::Connection;
use veilid_core::{VeilidAPI, VeilidUpdate, TypedKey, ValueSubkey, VeilidAPIResult, Target, OperationId, RouteId, DHTSchema, CryptoKind, DHTRecordDescriptor, KeyPair, ValueData};

#[tokio::test]
async fn test_something_interesting() {
    let (temp_dir, alice, alice_conn, alice_node) = setup().await;

    drop(temp_dir);
}

struct MockNode {
    receiver: Receiver<VeilidUpdate>,
    sender: Sender<VeilidUpdate>,
    api: VeilidAPI,
} 

impl MockNode {
    async fn new(state_path: String) -> MockNode {
        let (node_sender, updates): (
            Sender<veilid_core::VeilidUpdate>,
            Receiver<veilid_core::VeilidUpdate>,
        ) = unbounded();

        let update_callback = Arc::new(|_| { });
        let config_state_path = Arc::new(state_path.to_owned());
        let config_callback =
            Arc::new(move |key| ddcp::veilid_config::callback(config_state_path.to_string(), key));

        let api: veilid_core::VeilidAPI =
            veilid_core::api_startup(update_callback, config_callback).await.expect("api startup");
        MockNode { receiver: updates, sender: node_sender, api }
    }
}

#[async_trait]
impl Node for MockNode {
    fn clone_box(&self) -> Box<dyn Node> {
        Box::new(MockNode{
            receiver: self.receiver.clone(),
            sender: self.sender.clone(),
            api: self.api.clone(),
        })
    }

    fn api(&self) -> VeilidAPI {
        todo!()
    }

    async fn wait_for_network(&self) -> Result<()> {
        Ok(())
    }

    async fn recv_updates_async(&self) -> Result<VeilidUpdate> {
        self.receiver.recv_async().await.map_err(other_err)
    }

    async fn shutdown(&self) -> VeilidAPIResult<()> {
        self.api().detach().await?;
        self.api().shutdown().await;
        Ok(())
    }

    async fn app_call(&self, target: Target, message: Vec<u8>) -> VeilidAPIResult<Vec<u8>> {
        todo!();
    }

    async fn app_call_reply(&self, call_id: OperationId, message: Vec<u8>) -> VeilidAPIResult<()> {
        todo!();
    }

    fn import_remote_private_route(&self, blob: Vec<u8>) -> VeilidAPIResult<RouteId> {
        Ok(RouteId::default())
    }

    async fn new_private_route(&self) -> VeilidAPIResult<(RouteId, Vec<u8>)> {
        Ok((RouteId::default(), vec![]))
    }

    fn release_private_route(&self, route_id: RouteId) -> VeilidAPIResult<()> {
        Ok(())
    }

    async fn create_dht_record(
        &self,
        schema: DHTSchema,
        kind: Option<CryptoKind>,
    ) -> VeilidAPIResult<DHTRecordDescriptor> {
        todo!();
    }

    async fn open_dht_record(
        &self,
        key: TypedKey,
        writer: Option<KeyPair>,
    ) -> VeilidAPIResult<DHTRecordDescriptor> {
        todo!();
    }

    async fn close_dht_record(&self, key: TypedKey) -> VeilidAPIResult<()> {
        Ok(())
    }

    async fn delete_dht_record(&self, key: TypedKey) -> VeilidAPIResult<()> {
        Ok(())
    }

    async fn get_dht_value(
        &self,
        key: TypedKey,
        subkey: ValueSubkey,
        force_refresh: bool,
    ) -> VeilidAPIResult<Option<ValueData>> {
        todo!();
    }

    async fn set_dht_value(
        &self,
        key: TypedKey,
        subkey: ValueSubkey,
        data: Vec<u8>,
    ) -> VeilidAPIResult<Option<ValueData>> {
        Ok(None)
    }
}

async fn setup() -> (tempdir::TempDir, DDCP, Connection, Box<MockNode>) {
    let temp_dir = tempdir::TempDir::new("ddcp-test").expect("can create tempdir");
    let state_dir = path_as_string(temp_dir.as_ref().to_path_buf()).expect("stringable path");
    let conn = DDCP::new_connection(None, "target/debug/crsqlite").await.expect("db connection");
    let node = Box::new(MockNode::new(state_dir).await);
    let ddcp = DDCP::new_conn_node(conn.clone(), node.clone_box());
    (temp_dir, ddcp, conn, node)
}
