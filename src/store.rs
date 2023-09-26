use async_trait::async_trait;
use veilid_core::{
    CryptoKey, CryptoTyped, KeyPair, TypedKey, VeilidAPI, VeilidAPIError, VeilidAPIResult,
};

pub(crate) const TABLE_STORE_LOCAL: &'static str = "local";
pub(crate) const TABLE_STORE_LOCAL_COLUMNS: u32 = 2;

pub(crate) const TABLE_STORE_REMOTE: &'static str = "remote";
pub(crate) const TABLE_STORE_REMOTE_COLUMNS: u32 = 2;

#[async_trait]
pub trait Store: Send + Sync {
    fn clone_box(&self) -> Box<dyn Store>;
    async fn upgrade(&self) -> VeilidAPIResult<()>;
    async fn remove_remote(&self, name: String) -> VeilidAPIResult<()>;
    async fn remotes(&self) -> VeilidAPIResult<Vec<(String, CryptoTyped<CryptoKey>)>>;
    async fn load_local_keypair(&self, name: &str) -> VeilidAPIResult<Option<(TypedKey, KeyPair)>>;
    async fn load_remote(&self, name: &str)
        -> VeilidAPIResult<(Option<TypedKey>, Option<Vec<u8>>)>;
    async fn store_local_keypair(
        &self,
        name: &str,
        key: &TypedKey,
        owner: KeyPair,
    ) -> VeilidAPIResult<()>;
    async fn store_remote_key(&self, name: &str, key: &TypedKey) -> VeilidAPIResult<()>;
    async fn store_remote_site_id(&self, name: &str, site_id: Vec<u8>) -> VeilidAPIResult<()>;
}

pub(crate) struct VeilidStore(VeilidAPI);

impl VeilidStore {
    pub(crate) fn new(api: VeilidAPI) -> Box<dyn Store> {
        Box::new(VeilidStore(api))
    }
}

#[async_trait]
impl Store for VeilidStore {
    fn clone_box(&self) -> Box<dyn Store> {
        Box::new(VeilidStore(self.0.clone()))
    }
    async fn upgrade(&self) -> VeilidAPIResult<()> {
        // Upgrade columns if necessary
        let _ = self
            .0
            .table_store()?
            .open(TABLE_STORE_LOCAL, TABLE_STORE_LOCAL_COLUMNS)
            .await?;
        let _ = self
            .0
            .table_store()?
            .open(TABLE_STORE_REMOTE, TABLE_STORE_REMOTE_COLUMNS)
            .await?;
        Ok(())
    }
    async fn remove_remote(&self, name: String) -> VeilidAPIResult<()> {
        let db = self.0.table_store()?.open("remote", 1).await?;
        db.delete(0, name.as_bytes()).await?;
        Ok(())
    }
    async fn remotes(&self) -> VeilidAPIResult<Vec<(String, CryptoTyped<CryptoKey>)>> {
        let db = self.0.table_store()?.open("remote", 1).await?;
        let keys = db.get_keys(0).await?;
        let mut result = vec![];
        for db_key in keys.iter() {
            let name = std::str::from_utf8(db_key.as_slice())
                .map_err(|e| VeilidAPIError::Generic {
                    message: e.to_string(),
                })?
                .to_owned();
            if let (Some(remote_key), _) = self.load_remote(name.as_str()).await? {
                result.push((name, remote_key));
            }
        }
        Ok(result)
    }
    async fn load_local_keypair(&self, name: &str) -> VeilidAPIResult<Option<(TypedKey, KeyPair)>> {
        let db = self
            .0
            .table_store()?
            .open(TABLE_STORE_LOCAL, TABLE_STORE_LOCAL_COLUMNS)
            .await?;
        let key = db.load_json::<TypedKey>(0, name.as_bytes()).await?;
        let owner = db.load_json::<KeyPair>(1, name.as_bytes()).await?;
        Ok(match (key, owner) {
            (Some(k), Some(o)) => Some((k, o)),
            _ => None,
        })
    }
    async fn load_remote(
        &self,
        name: &str,
    ) -> VeilidAPIResult<(Option<TypedKey>, Option<Vec<u8>>)> {
        let db = self
            .0
            .table_store()?
            .open(TABLE_STORE_REMOTE, TABLE_STORE_REMOTE_COLUMNS)
            .await?;
        let key = db.load_json::<TypedKey>(0, name.as_bytes()).await?;
        let site_id = db.load_json::<Vec<u8>>(1, name.as_bytes()).await?;
        Ok((key, site_id))
    }
    async fn store_local_keypair(
        &self,
        name: &str,
        key: &TypedKey,
        owner: KeyPair,
    ) -> VeilidAPIResult<()> {
        let db = self
            .0
            .table_store()?
            .open(TABLE_STORE_LOCAL, TABLE_STORE_LOCAL_COLUMNS)
            .await?;
        db.store_json(0, name.as_bytes(), key).await?;
        db.store_json(1, name.as_bytes(), &owner).await
    }
    async fn store_remote_key(&self, name: &str, key: &TypedKey) -> VeilidAPIResult<()> {
        let db = self
            .0
            .table_store()?
            .open(TABLE_STORE_REMOTE, TABLE_STORE_REMOTE_COLUMNS)
            .await?;
        db.store_json(0, name.as_bytes(), key).await
    }
    async fn store_remote_site_id(&self, name: &str, site_id: Vec<u8>) -> VeilidAPIResult<()> {
        let db = self
            .0
            .table_store()?
            .open(TABLE_STORE_REMOTE, TABLE_STORE_REMOTE_COLUMNS)
            .await?;
        db.store_json(1, name.as_bytes(), &site_id).await
    }
}
