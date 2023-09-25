use async_trait::async_trait;

use flume::Receiver;
use veilid_core::{
    DHTRecordDescriptor, KeyPair, OperationId, RouteId, RoutingContext, Target, TypedKey,
    ValueData, ValueSubkey, VeilidAPI, VeilidAPIResult, VeilidUpdate, DHTSchema, CryptoKind,
};

use crate::{
    error::{Error, Result},
    other_err,
};

#[async_trait]
pub trait Node: Send + Sync {
    fn clone_box(&self) -> Box<dyn Node>;

    fn api(&self) -> VeilidAPI;

    async fn wait_for_network(&self) -> Result<()>;

    async fn recv_updates_async(&self) -> Result<VeilidUpdate>;

    async fn shutdown(&self) -> VeilidAPIResult<()>;

    // AppCall operations

    async fn app_call(&self, target: Target, message: Vec<u8>) -> VeilidAPIResult<Vec<u8>>;
    async fn app_call_reply(&self, call_id: OperationId, message: Vec<u8>) -> VeilidAPIResult<()>;

    // Routing operations

    fn import_remote_private_route(&self, blob: Vec<u8>) -> VeilidAPIResult<RouteId>;
    async fn new_private_route(&self) -> VeilidAPIResult<(RouteId, Vec<u8>)>;
    fn release_private_route(&self, route_id: RouteId) -> VeilidAPIResult<()>;

    // DHT operations

    async fn create_dht_record(
        &self,
        schema: DHTSchema,
        kind: Option<CryptoKind>,
    ) -> VeilidAPIResult<DHTRecordDescriptor>;

    async fn open_dht_record(
        &self,
        key: TypedKey,
        writer: Option<KeyPair>,
    ) -> VeilidAPIResult<DHTRecordDescriptor>;

    async fn close_dht_record(&self, key: TypedKey) -> VeilidAPIResult<()>;

    async fn delete_dht_record(&self, key: TypedKey) -> VeilidAPIResult<()>;

    async fn get_dht_value(
        &self,
        key: TypedKey,
        subkey: ValueSubkey,
        force_refresh: bool,
    ) -> VeilidAPIResult<Option<ValueData>>;
    async fn set_dht_value(
        &self,
        key: TypedKey,
        subkey: ValueSubkey,
        data: Vec<u8>,
    ) -> VeilidAPIResult<Option<ValueData>>;
}

pub struct VeilidNode {
    routing_context: RoutingContext,
    updates: Receiver<VeilidUpdate>,
}

impl VeilidNode {
    pub fn new(routing_context: RoutingContext, updates: Receiver<VeilidUpdate>) -> Box<dyn Node> {
        Box::new(VeilidNode {
            routing_context,
            updates,
        })
    }
}

#[async_trait]
impl Node for VeilidNode {
    fn clone_box(&self) -> Box<dyn Node> {
        Box::new(VeilidNode {
            routing_context: self.routing_context.clone(),
            updates: self.updates.clone(),
        })
    }

    fn api(&self) -> VeilidAPI {
        self.routing_context.api()
    }

    async fn wait_for_network(&self) -> Result<()> {
        // Wait for network to be up
        loop {
            let res = self.updates.recv_async().await;
            match res {
                Ok(VeilidUpdate::Attachment(attachment)) => {
                    eprintln!("{:?}", attachment);
                    if attachment.public_internet_ready {
                        return Ok(());
                    }
                }
                Ok(VeilidUpdate::Config(_)) => {}
                Ok(VeilidUpdate::Log(_)) => {}
                Ok(VeilidUpdate::Network(_)) => {}
                Ok(u) => {
                    eprintln!("{:?}", u);
                }
                Err(e) => {
                    return Err(Error::Other(e.to_string()));
                }
            };
        }
    }

    async fn recv_updates_async(&self) -> Result<VeilidUpdate> {
        self.updates.recv_async().await.map_err(other_err)
    }

    async fn shutdown(&self) -> VeilidAPIResult<()> {
        self.api().detach().await?;
        self.api().shutdown().await;
        Ok(())
    }

    async fn app_call(&self, target: Target, message: Vec<u8>) -> VeilidAPIResult<Vec<u8>> {
        self.routing_context.app_call(target, message).await
    }

    async fn app_call_reply(&self, call_id: OperationId, message: Vec<u8>) -> VeilidAPIResult<()> {
        self.routing_context
            .api()
            .app_call_reply(call_id, message)
            .await
    }

    fn import_remote_private_route(&self, blob: Vec<u8>) -> VeilidAPIResult<RouteId> {
        self.routing_context.api().import_remote_private_route(blob)
    }

    async fn new_private_route(&self) -> VeilidAPIResult<(RouteId, Vec<u8>)> {
        self.routing_context.api().new_private_route().await
    }

    fn release_private_route(&self, route_id: RouteId) -> VeilidAPIResult<()> {
        self.routing_context.api().release_private_route(route_id)
    }

    async fn create_dht_record(
        &self,
        schema: DHTSchema,
        kind: Option<CryptoKind>,
    ) -> VeilidAPIResult<DHTRecordDescriptor> {
        self.routing_context.create_dht_record(schema, kind).await
    }

    async fn open_dht_record(
        &self,
        key: TypedKey,
        writer: Option<KeyPair>,
    ) -> VeilidAPIResult<DHTRecordDescriptor> {
        self.routing_context.open_dht_record(key, writer).await
    }

    async fn close_dht_record(&self, key: TypedKey) -> VeilidAPIResult<()> {
        self.routing_context.close_dht_record(key).await
    }

    async fn delete_dht_record(&self, key: TypedKey) -> VeilidAPIResult<()> {
        self.routing_context.delete_dht_record(key).await
    }

    async fn get_dht_value(
        &self,
        key: TypedKey,
        subkey: ValueSubkey,
        force_refresh: bool,
    ) -> VeilidAPIResult<Option<ValueData>> {
        self.routing_context
            .get_dht_value(key, subkey, force_refresh)
            .await
    }

    async fn set_dht_value(
        &self,
        key: TypedKey,
        subkey: ValueSubkey,
        data: Vec<u8>,
    ) -> VeilidAPIResult<Option<ValueData>> {
        self.routing_context.set_dht_value(key, subkey, data).await
    }
}
