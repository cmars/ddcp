use async_trait::async_trait;

use flume::Receiver;
use tracing::{info, warn};
use veilid_core::{
    CryptoKind, DHTRecordDescriptor, DHTSchema, KeyPair, OperationId, RouteId, RoutingContext,
    Target, TypedKey, ValueData, ValueSubkey, VeilidAPIResult, VeilidUpdate, SharedSecret, CRYPTO_KIND_VLD0,
};

use crate::{
    error::{Error, Result},
    other_err,
    store::{Store, VeilidStore},
};

#[async_trait]
pub trait Node: Send + Sync {
    fn clone_box(&self) -> Box<dyn Node>;

    fn store(&self) -> &dyn Store;

    fn generate_peer_secret(&self) -> Result<SharedSecret>;

    fn encrypt_message(&self, plaintext: &[u8], secret: SharedSecret) -> Result<Vec<u8>>;

    fn decrypt_message(&self, msg_bytes: &[u8], secret: SharedSecret) -> Result<Vec<u8>>;

    async fn wait_for_network(&self) -> Result<()>;

    async fn shutdown(&self, maybe_dht: Option<DHTRecordDescriptor>) -> VeilidAPIResult<()>;

    async fn recv_updates_async(&self) -> Result<VeilidUpdate>;

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
        &mut self,
        key: TypedKey,
        subkey: ValueSubkey,
        data: Vec<u8>,
    ) -> VeilidAPIResult<Option<ValueData>>;
}

pub struct VeilidNode {
    table_store: Box<dyn Store>,
    routing_context: RoutingContext,
    updates: Receiver<VeilidUpdate>,
}

impl VeilidNode {
    pub fn new(routing_context: RoutingContext, updates: Receiver<VeilidUpdate>) -> Box<dyn Node> {
        Box::new(VeilidNode {
            table_store: VeilidStore::new(routing_context.api()),
            routing_context,
            updates,
        })
    }
}

#[cfg(feature="todo")]
#[async_trait]
impl Node for VeilidNode {
    fn clone_box(&self) -> Box<dyn Node> {
        Box::new(VeilidNode {
            table_store: self.table_store.clone_box(),
            routing_context: self.routing_context.clone(),
            updates: self.updates.clone(),
        })
    }

    fn store(&self) -> &dyn Store {
        self.table_store.as_ref()
    }

    fn generate_peer_secret(&self) -> Result<SharedSecret> {
        Ok(vld0_crypto_system(&self.routing_context.api().crypto()?)?.random_shared_secret())
    }

    fn encrypt_message(&self, plaintext: &[u8], secret: SharedSecret) -> Result<Vec<u8>> {
        let result = match secret {
            Some(ref s) => {
                let c = vld0_crypto_system(&self.routing_context.api().crypto()?)?;
                let nonce = c.random_nonce();
                let ciphertext = c.encrypt_aead(plaintext, &nonce, s, None)?;
                let mut result = nonce.to_vec();
                result.extend(ciphertext);
                result
            }
            None => plaintext,
        };
        Ok(result)
    }

    fn decrypt_message(&self, msg_bytes: &[u8], secret: SharedSecret) -> Result<Vec<u8>> {
        let plaintext = match secret {
            Some(ref s) => {
                let c = vld0_crypto_system(&self.routing_context.api().crypto()?)?;
                let nonce = Nonce::new(msg_bytes[0..24].try_into().map_err(other_err)?);
                let ciphertext = &msg_bytes[24..];
                c.decrypt_aead(ciphertext, &nonce, s, None)?
            }
            None => msg_bytes.to_vec(),
        };
        Ok(plaintext)
    }

    async fn wait_for_network(&self) -> Result<()> {
        // Wait for network to be up
        loop {
            let res = self.updates.recv_async().await;
            match res {
                Ok(VeilidUpdate::Attachment(attachment)) => {
                    if attachment.public_internet_ready {
                        info!(
                            state = attachment.state.to_string(),
                            public_internet_ready = attachment.public_internet_ready,
                            "Connected"
                        );
                        return Ok(());
                    }
                    info!(
                        state = attachment.state.to_string(),
                        public_internet_ready = attachment.public_internet_ready,
                        "Waiting for network"
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    return Err(Error::Other(e.to_string()));
                }
            };
        }
    }

    async fn recv_updates_async(&self) -> Result<VeilidUpdate> {
        self.updates.recv_async().await.map_err(other_err)
    }

    async fn shutdown(&self, maybe_dht: Option<DHTRecordDescriptor>) -> VeilidAPIResult<()> {
        // Upgrade table store columns. Idempotent, expands columns if a new
        // release adds a column.
        self.table_store.upgrade().await?;

        // Attempt to close DHT record in local store.
        if let Some(dht) = maybe_dht {
            if let Err(e) = self.close_dht_record(dht.key().to_owned()).await {
                warn!(
                    key = dht.key().to_string(),
                    err = format!("{:?}", e),
                    "Failed to close DHT record"
                );
            }
        }

        // Graceful shutdown of Veilid node.
        self.routing_context.api().detach().await?;
        self.routing_context.api().shutdown().await;
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
        &mut self,
        key: TypedKey,
        subkey: ValueSubkey,
        data: Vec<u8>,
    ) -> VeilidAPIResult<Option<ValueData>> {
        self.routing_context.set_dht_value(key, subkey, data).await
    }
}

fn vld0_crypto_system(crypto: &Crypto) -> Result<CryptoSystemVersion> {
    Ok(crypto
        .get(CRYPTO_KIND_VLD0)
        .ok_or(other_err("missing VLD0"))?)
}
