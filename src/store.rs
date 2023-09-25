use veilid_core::{VeilidAPI, VeilidAPIResult, TypedKey, KeyPair};

pub(crate) const TABLE_STORE_LOCAL: &'static str = "local";
pub(crate) const TABLE_STORE_LOCAL_COLUMNS: u32 = 2;

pub(crate) const TABLE_STORE_REMOTE: &'static str = "remote";
pub(crate) const TABLE_STORE_REMOTE_COLUMNS: u32 = 2;

pub(crate) async fn load_local_keypair(
    api: VeilidAPI,
    name: &str,
) -> VeilidAPIResult<Option<(TypedKey, KeyPair)>> {
    let db = api
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

pub(crate) async fn load_remote(
    api: VeilidAPI,
    name: &str,
) -> VeilidAPIResult<(Option<TypedKey>, Option<Vec<u8>>)> {
    let db = api
        .table_store()?
        .open(TABLE_STORE_REMOTE, TABLE_STORE_REMOTE_COLUMNS)
        .await?;
    let key = db.load_json::<TypedKey>(0, name.as_bytes()).await?;
    let site_id = db.load_json::<Vec<u8>>(1, name.as_bytes()).await?;
    Ok((key, site_id))
}

pub(crate) async fn store_local_keypair(
    api: VeilidAPI,
    name: &str,
    key: &TypedKey,
    owner: KeyPair,
) -> VeilidAPIResult<()> {
    let db = api
        .table_store()?
        .open(TABLE_STORE_LOCAL, TABLE_STORE_LOCAL_COLUMNS)
        .await?;
    db.store_json(0, name.as_bytes(), key).await?;
    db.store_json(1, name.as_bytes(), &owner).await
}

pub(crate) async fn store_remote_key(api: VeilidAPI, name: &str, key: &TypedKey) -> VeilidAPIResult<()> {
    let db = api
        .table_store()?
        .open(TABLE_STORE_REMOTE, TABLE_STORE_REMOTE_COLUMNS)
        .await?;
    db.store_json(0, name.as_bytes(), key).await
}

pub(crate) async fn store_remote_site_id(api: VeilidAPI, name: &str, site_id: Vec<u8>) -> VeilidAPIResult<()> {
    let db = api
        .table_store()?
        .open(TABLE_STORE_REMOTE, TABLE_STORE_REMOTE_COLUMNS)
        .await?;
    db.store_json(1, name.as_bytes(), &site_id).await
}
