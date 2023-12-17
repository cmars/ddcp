use std::collections::HashMap;

use veilid_core::FromStr;
use veilid_core::{
    CryptoSystemVLD0, CryptoTyped, DHTRecordDescriptor, DHTSchemaDFLT, KeyPair, PublicKey, RouteId,
    RoutingContext, SharedSecret, TableDB, TableStore, TypedKey, TypedKeyPair, ValueSubkey,
    VeilidAPI, CRYPTO_KIND_VLD0,
};

use crate::{error::Result, other_err};

const DHT_N_SUBKEYS: u16 = 3;
const DHT_SUBKEY_PRIVATE_ROUTE: ValueSubkey = 0;
const DHT_SUBKEY_SITE_ID: ValueSubkey = 1;
const DHT_SUBKEY_DB_VERSION: ValueSubkey = 2;

const TABLE_STORE_LOCAL_N_COLUMNS: u32 = 5;
const TABLE_STORE_LOCAL_COLUMN_SHARED_SECRET: u32 = 0;
const TABLE_STORE_LOCAL_COLUMN_DHT_KEY: u32 = 1;
const TABLE_STORE_LOCAL_COLUMN_DHT_OWNER_KEYPAIR: u32 = 2;

const TABLE_STORE_REMOTES_N_COLUMNS: u32 = 3;
const TABLE_STORE_REMOTES_COLUMN_DHT_KEY: u32 = 0;

pub struct Sovereign {
    dht_key: TypedKey,
    dht_owner_keypair: CryptoTyped<KeyPair>,

    dht: Option<DHTRecordDescriptor>,
    route_id: Option<RouteId>,
}

impl Sovereign {
    async fn open_db(ts: &TableStore) -> Result<TableDB> {
        let db = ts
            .open("ddcp_conclave_local", TABLE_STORE_LOCAL_N_COLUMNS)
            .await?;
        Ok(db)
    }

    pub async fn init(routing_context: &RoutingContext) -> Result<Sovereign> {
        let ts = routing_context.api().table_store()?;
        let db = Self::open_db(&ts).await?;
        let crypto = routing_context
            .api()
            .crypto()?
            .get(CRYPTO_KIND_VLD0)
            .ok_or(other_err("missing VLD0 cryptosystem"))?;

        // create DHT
        let new_dht = routing_context
            .create_dht_record(
                veilid_core::DHTSchema::DFLT(DHTSchemaDFLT {
                    o_cnt: DHT_N_SUBKEYS,
                }),
                None,
            )
            .await?;
        let dht_owner_keypair = KeyPair::new(
            new_dht.owner().to_owned(),
            new_dht
                .owner_secret()
                .ok_or(other_err("expected dht owner secret"))?
                .to_owned(),
        );
        routing_context
            .close_dht_record(new_dht.key().to_owned())
            .await?;

        // write these to db
        db.store_json(TABLE_STORE_LOCAL_COLUMN_DHT_KEY, &[], new_dht.key())
            .await?;
        db.store_json(
            TABLE_STORE_LOCAL_COLUMN_DHT_OWNER_KEYPAIR,
            &[],
            &dht_owner_keypair,
        )
        .await?;

        // read back from storage; redundant, but asserts data integrity
        Self::load(routing_context)
            .await?
            .ok_or(other_err("failed to initialize sovereign identity"))
    }

    pub async fn load(routing_context: &RoutingContext) -> Result<Option<Sovereign>> {
        let ts = routing_context.api().table_store()?;
        let db = Self::open_db(&ts).await?;
        let Some(dht_key) = db
            .load_json::<TypedKey>(TABLE_STORE_LOCAL_COLUMN_DHT_KEY, &[])
            .await?
        else {
            return Ok(None);
        };
        let Some(dht_owner_keypair) = db
            .load_json::<TypedKeyPair>(TABLE_STORE_LOCAL_COLUMN_DHT_OWNER_KEYPAIR, &[])
            .await?
        else {
            return Ok(None);
        };

        let dht = routing_context
            .open_dht_record(dht_key.clone(), Some(dht_owner_keypair.value.clone()))
            .await?;
        Ok(Some(Sovereign {
            dht_key,
            dht_owner_keypair,
            dht: Some(dht),
            route_id: None,
        }))
    }

    pub async fn announce(&mut self, routing_context: &RoutingContext) -> Result<()> {
        if let Some(route_id) = self.route_id {
            routing_context.api().release_private_route(route_id)?;
        }
        let (route_id, route_blob) = routing_context.api().new_private_route().await?;
        self.route_id = Some(route_id);
        routing_context
            .set_dht_value(self.dht_key, DHT_SUBKEY_PRIVATE_ROUTE, route_blob)
            .await?;
        Ok(())
    }

    async fn close(&mut self, routing_context: &RoutingContext) -> Result<()> {
        if let Some(dht) = self.dht.take() {
            routing_context
                .close_dht_record(dht.key().to_owned())
                .await?;
        }
        if let Some(route_id) = self.route_id.take() {
            routing_context.api().release_private_route(route_id)?;
        }
        Ok(())
    }
}

pub struct Peer {
    name: String,
    dht_public_key: TypedKey,

    dht: Option<DHTRecordDescriptor>,
    route_id: Option<RouteId>,
}

impl Peer {
    async fn open_db(ts: &TableStore) -> Result<TableDB> {
        let db = ts
            .open("ddcp_conclave_remotes", TABLE_STORE_REMOTES_N_COLUMNS)
            .await?;
        Ok(db)
    }

    pub async fn new(
        api: &VeilidAPI,
        name: &str,
        dht_public_key: &str,
    ) -> Result<Peer> {
        let ts = api.table_store()?;
        let db = Self::open_db(&ts).await?;
        let db_key = name.as_bytes().to_vec();

        // Parse and store dht key
        let dht_key = TypedKey::from_str(dht_public_key)?;
        db.store_json(
            TABLE_STORE_REMOTES_COLUMN_DHT_KEY,
            db_key.as_slice(),
            &dht_key,
        )
        .await?;

        Self::load(name, &db, &db_key).await
    }

    pub async fn load_all(api: &VeilidAPI) -> Result<HashMap<String, Peer>> {
        let ts = api.table_store()?;
        let db = Self::open_db(&ts).await?;
        let mut remotes = HashMap::new();
        for remote_key in db
            .get_keys(TABLE_STORE_REMOTES_COLUMN_DHT_KEY)
            .await?
            .iter()
        {
            let remote_name = String::from_utf8(remote_key.to_owned()).map_err(other_err)?;
            let peer = Peer::load(&remote_name, &db, remote_key).await?;
            remotes.insert(remote_name, peer);
        }
        Ok(remotes)
    }

    async fn load(name: &str, db: &TableDB, db_key: &Vec<u8>) -> Result<Peer> {
        let dht_public_key = db
            .load_json::<TypedKey>(TABLE_STORE_REMOTES_COLUMN_DHT_KEY, db_key)
            .await?
            .ok_or(other_err("remote peer missing dht key"))?;
        Ok(Peer {
            name: name.to_owned(),
            dht_public_key,
            dht: None,
            route_id: None,
        })
    }

    pub async fn refresh(&mut self, routing_context: &RoutingContext) -> Result<()> {
        self.refresh_dht(routing_context).await?;
        self.refresh_route(routing_context).await?;
        Ok(())
    }

    async fn refresh_dht(&mut self, routing_context: &RoutingContext) -> Result<()> {
        if let Some(dht) = self.dht.as_ref() {
            routing_context
                .close_dht_record(dht.key().to_owned())
                .await?;
        };
        self.dht = Some(
            routing_context
                .open_dht_record(self.dht_public_key, None)
                .await?,
        );
        Ok(())
    }

    async fn refresh_route(&mut self, routing_context: &RoutingContext) -> Result<()> {
        if let Some(route_id) = self.route_id {
            routing_context.api().release_private_route(route_id)?;
        }

        match routing_context
            .get_dht_value(self.dht_public_key, DHT_SUBKEY_PRIVATE_ROUTE, true)
            .await?
        {
            Some(route_data) => {
                self.route_id = Some(
                    routing_context
                        .api()
                        .import_remote_private_route(route_data.data().to_vec())?,
                );
            }
            None => return Err(other_err("remote DHT missing route")),
        }
        Ok(())
    }

    async fn close(&mut self, routing_context: &RoutingContext) -> Result<()> {
        if let Some(dht) = self.dht.take() {
            routing_context
                .close_dht_record(dht.key().to_owned())
                .await?;
        }
        if let Some(route_id) = self.route_id.take() {
            routing_context.api().release_private_route(route_id)?;
        }
        Ok(())
    }
}

/// A Conclave defines a local sovereign identity and a group of peers which
/// share a replication secret.
pub struct Conclave {
    routing_context: RoutingContext,

    sovereign: Sovereign,
    remotes: HashMap<String, Peer>,
}

impl Conclave {
    pub async fn new(routing_context: RoutingContext) -> Result<Conclave> {
        let mut sovereign = match Sovereign::load(&routing_context).await? {
            Some(sov) => sov,
            None => Sovereign::init(&routing_context).await?,
        };

        let mut remotes = Peer::load_all(&routing_context.api()).await?;

        Ok(Conclave {
            routing_context,
            sovereign,
            remotes,
        })
    }

    pub async fn refresh(&mut self) -> Result<()> {
        self.sovereign.announce(&self.routing_context).await?;
        for peer in self.remotes.values_mut().into_iter() {
            peer.refresh(&self.routing_context).await?;
        }
        Ok(())
    }

    pub fn sovereign(&self) -> &Sovereign {
        return &self.sovereign;
    }

    pub fn peer(&self, name: &str) -> Option<&Peer> {
        return self.remotes.get(name);
    }

    pub async fn set_peer(&mut self, mut peer: Peer) -> Result<()> {
        self.remotes.insert(peer.name.clone(), peer);
        Ok(())
    }

    pub fn peers<'a>(&'a self) -> std::collections::hash_map::Values<'a, String, Peer> {
        self.remotes.values().into_iter()
    }

    pub async fn close(mut self) -> Result<()> {
        self.sovereign.close(&self.routing_context).await?;
        for peer in self.remotes.values_mut().into_iter() {
            peer.close(&self.routing_context).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use veilid_core::Sequencing;

    use crate::tests::api::{setup_api, teardown_api, TEST_API_MUTEX};

    use super::*;

    #[tokio::test]
    async fn basic() {
        let _lock = TEST_API_MUTEX.lock().expect("lock");

        let api = setup_api().await;
        let routing_context = api
            .routing_context().expect("routing context")
            .with_sequencing(Sequencing::PreferOrdered)
            .with_default_safety()
            .expect("ok");
        let mut ccl = Conclave::new(routing_context).await.expect("ok");
        assert_eq!(ccl.peers().len(), 0);
        let peer = Peer::new(
            &api,
            "bob",
            "VLD0:7lxDEabK_qgjbe38RtBa3IZLrud84P6NhGP-pRTZzdQ",
        )
        .await
        .expect("new peer");
        ccl.set_peer(peer).await.expect("set peer");
        assert_eq!(ccl.peers().len(), 1);
        ccl.close().await.expect("ok");
        teardown_api(api).await;
    }
}
