use std::{io, sync::Arc, time::Duration};

use flume::{unbounded, Receiver, Sender};
use rusqlite::{params, OptionalExtension};
use tokio::{select, signal, sync::oneshot, task::JoinHandle};
use tokio_rusqlite::Connection;
use veilid_core::{
    CryptoKey, CryptoTyped, DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, FromStr, KeyPair,
    RoutingContext, Sequencing, Target, TypedKey, ValueSubkey, VeilidAPI, VeilidAPIError,
    VeilidAPIResult, VeilidUpdate,
};

pub mod cli;
mod proto;
mod veilid_config;

use proto::{Request, Response};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("db error: {0}")]
    DB(#[from] tokio_rusqlite::Error),
    #[error("io error: {0}")]
    IO(#[from] io::Error),
    #[error("veilid api error: {0}")]
    VeilidAPI(#[from] VeilidAPIError),
    #[error("prorotcol error: {0}")]
    Protocol(#[from] proto::Error),
    #[error("utf-8 encoding error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("other: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct DDCP {
    conn: Connection,
    api: VeilidAPI,
    updates: Receiver<VeilidUpdate>,
    routing_context: RoutingContext,
    dht_keypair: Option<DHTRecordDescriptor>,
}

const LOCAL_KEYPAIR_NAME: &'static str = "__local";

const SUBKEY_SITE_ID: ValueSubkey = 0;
const SUBKEY_DB_VERSION: ValueSubkey = 1;
const SUBKEY_PRIVATE_ROUTE: ValueSubkey = 2;

const CRSQL_TRACKED_TAG_WHOLE_DATABASE: i32 = 0;

const CRSQL_TRACKED_EVENT_RECEIVE: i32 = 0;
//const CRSQL_TRACKED_EVENT_SEND: i32 = 1;
const CRSQL_TRACKED_EVENT_DDCP_MERGE: i32 = 1000;

impl DDCP {
    pub async fn new(db_path: &str, state_path: &str, ext_path: &str) -> Result<DDCP> {
        let conn = DDCP::new_connection(db_path, ext_path).await?;
        let (api, updates) = DDCP::new_veilid_node(state_path).await?;
        let routing_context = api
            .routing_context()
            .with_sequencing(Sequencing::EnsureOrdered)
            .with_privacy()?;
        Ok(DDCP {
            conn,
            api,
            updates,
            routing_context,
            dht_keypair: None,
        })
    }

    async fn new_connection(db_path: &str, ext_path: &str) -> Result<Connection> {
        let conn = Connection::open(db_path).await?;
        let load_ext_path = ext_path.to_owned();
        conn.call(move |c| {
            unsafe {
                c.load_extension_enable()?;
                let r = c.load_extension(load_ext_path, Some("sqlite3_crsqlite_init"))?;
                c.load_extension_disable()?;
                r
            };
            Ok(())
        })
        .await?;
        Ok(conn)
    }

    async fn new_veilid_node(state_path: &str) -> Result<(VeilidAPI, Receiver<VeilidUpdate>)> {
        // Veilid API state channel
        let (node_sender, updates): (
            Sender<veilid_core::VeilidUpdate>,
            Receiver<veilid_core::VeilidUpdate>,
        ) = unbounded();

        // Start up Veilid core
        let update_callback = Arc::new(move |change: veilid_core::VeilidUpdate| {
            let _ = node_sender.send(change);
        });
        let config_state_path = Arc::new(state_path.to_owned());
        let config_callback =
            Arc::new(move |key| veilid_config::callback(config_state_path.to_string(), key));
        let api: veilid_core::VeilidAPI =
            veilid_core::api_startup(update_callback, config_callback).await?;
        api.attach().await?;
        Ok((api, updates))
    }

    pub async fn wait_for_network(&self) -> Result<()> {
        // Wait for network to be up
        async {
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
        .await
    }

    pub async fn init(&mut self) -> Result<()> {
        // Initialize tracker table
        self.init_db().await?;
        // Load or create DHT key
        let local_dht = self.dht_keypair(LOCAL_KEYPAIR_NAME).await?;
        println!("{}", local_dht.key().to_string());
        self.dht_keypair = Some(local_dht);
        Ok(())
    }

    async fn init_db(&self) -> Result<()> {
        self.conn
            .call(|conn| {
                conn.execute(
                    "
                    create table if not exists ddcp_remote_changes (
                        \"table\" text not null,
                        pk text not null,
                        cid text not null,
                        val any,
                        col_version integer not null,
                        db_version integer not null,
                        site_id blob not null,
                        cl integer not null,
                        seq integer not null);
                    ",
                    [],
                )
            })
            .await?;
        Ok(())
    }

    pub async fn push(&mut self) -> Result<()> {
        // Load or create DHT key
        let local_dht = self.dht_keypair(LOCAL_KEYPAIR_NAME).await?;
        println!("{}", local_dht.key().to_string());
        self.dht_keypair = Some(local_dht.clone());

        let key = local_dht.key().to_owned();

        let (site_id, db_version) = status(&self.conn).await?;

        // Push current db state
        self.routing_context
            .set_dht_value(key, SUBKEY_SITE_ID, site_id)
            .await?;
        self.routing_context
            .set_dht_value(key, SUBKEY_DB_VERSION, db_version.to_be_bytes().to_vec())
            .await?;

        Ok(())
    }

    pub async fn fetch(&mut self, name: &str) -> Result<()> {
        // Get latest status from DHT
        let (site_id, db_version, route_blob) = self.remote_status(name).await?;

        // Do we need to fetch newer versions?
        let tracked_version = match self
            .conn
            .call(move |conn| {
                conn.query_row(
                    "
select max(version) from crsql_tracked_peers
where site_id = ? and event = ? and version is not null",
                    params![site_id, CRSQL_TRACKED_EVENT_RECEIVE],
                    |row| row.get::<usize, Option<i64>>(0),
                )
                .optional()
            })
            .await?
        {
            Some(Some(tracked_version)) => {
                if db_version <= tracked_version {
                    eprintln!("remote {} is up to date", name);
                    return Ok(());
                }
                tracked_version
            }
            _ => -1,
        };

        let route_id = self.api.import_remote_private_route(route_blob)?;
        let msg_bytes = self
            .routing_context
            .app_call(
                Target::PrivateRoute(route_id),
                proto::encode_request_message(Request::Changes {
                    since_db_version: tracked_version,
                })?,
            )
            .await?;
        let resp = proto::decode_response_message(msg_bytes.as_slice())?;

        if let Response::Changes { site_id, changes } = resp {
            self.stage(site_id, changes).await?;
        }

        Ok(())
    }

    async fn remote_status(&mut self, name: &str) -> Result<(Vec<u8>, i64, Vec<u8>)> {
        let (prior_dht_key, prior_site_id) = load_remote(&self.api, name).await?;
        let dht = match prior_dht_key {
            Some(key) => self.routing_context.open_dht_record(key, None).await?,
            None => return Err(other_err(format!("remote {} not found", name))),
        };
        let maybe_site_id = self
            .routing_context
            .get_dht_value(dht.key().to_owned(), SUBKEY_SITE_ID, true)
            .await?;
        let maybe_db_version = self
            .routing_context
            .get_dht_value(dht.key().to_owned(), SUBKEY_DB_VERSION, true)
            .await?;
        let maybe_route_blob = self
            .routing_context
            .get_dht_value(dht.key().to_owned(), SUBKEY_PRIVATE_ROUTE, true)
            .await?;

        match (prior_site_id, &maybe_site_id) {
            (None, Some(site_id)) => {
                store_remote_site_id(&self.api, name, site_id.data().to_owned()).await?;
            }
            _ => {}
        };

        Ok(match (maybe_site_id, maybe_db_version, maybe_route_blob) {
            (Some(sid), Some(dbv), Some(rt)) => {
                let dbv_arr: [u8; 8] = dbv.data().try_into().map_err(other_err)?;
                (
                    sid.data().to_owned(),
                    i64::from_be_bytes(dbv_arr),
                    rt.data().to_owned(),
                )
            }
            _ => return Err(other_err(format!("remote {} not available", name))),
        })
    }

    async fn stage(&mut self, site_id: Vec<u8>, changes: Vec<proto::Change>) -> Result<()> {
        self.conn
            .call(move |conn| {
                let tx = conn.transaction()?;
                let mut max_db_version = -1;
                let mut stmt = tx.prepare(
                    "
insert into ddcp_remote_changes (
    \"table\", pk, cid, val, col_version, db_version, site_id, cl, seq
)
values (?, ?, ?, ?, ?, ?, ?, ?, ?);",
                )?;
                for change in changes.iter() {
                    if change.db_version > max_db_version {
                        max_db_version = change.db_version;
                    }
                    stmt.execute(params![
                        &change.table,
                        &change.pk,
                        &change.cid,
                        &change.val,
                        &change.col_version,
                        &change.db_version,
                        &site_id,
                        &change.cl,
                        &change.seq,
                    ])?;
                }
                tx.execute(
                    "
insert into crsql_tracked_peers (site_id, version, tag, event)
values (?, ?, ?, ?)
on conflict do update set version = excluded.version",
                    params![
                        site_id,
                        max_db_version,
                        CRSQL_TRACKED_TAG_WHOLE_DATABASE,
                        CRSQL_TRACKED_EVENT_RECEIVE
                    ],
                )?;
                drop(stmt);
                tx.commit()
            })
            .await?;
        Ok(())
    }

    pub async fn merge(&mut self, name: &str) -> Result<()> {
        let site_id = if let (Some(_), Some(site_id)) = load_remote(&self.api, name).await? {
            site_id
        } else {
            return Err(other_err(format!(
                "cannot merge from {}; no site_id: try fetching first",
                name
            )));
        };
        self.conn
            .call(move |conn| {
                let tx = conn.transaction()?;
                let max_db_version = match tx
                    .query_row(
                        "
select cast(max(version) as integer) as max_version from crsql_tracked_peers
where site_id = ? and tag = ? and event = ?",
                        params![
                            site_id,
                            CRSQL_TRACKED_TAG_WHOLE_DATABASE,
                            CRSQL_TRACKED_EVENT_DDCP_MERGE
                        ],
                        |row| row.get::<usize, Option<i64>>(0),
                    )
                    .optional()?
                {
                    Some(Some(version)) => version,
                    _ => -1,
                };
                tx.execute(
                    "
insert into crsql_changes (
    \"table\", pk, cid, val, col_version, db_version, site_id, cl, seq)
select
    \"table\", pk, cid, val, col_version, db_version, site_id, cl, seq
from ddcp_remote_changes
where site_id = ?
and db_version > ?",
                    params![site_id, max_db_version],
                )?;
                tx.execute(
                    "
insert into crsql_tracked_peers (
    site_id, version, tag, event)
select site_id, max(db_version), ?, ?
from ddcp_remote_changes
where site_id = ?
order by db_version desc
limit 1",
                    params![CRSQL_TRACKED_TAG_WHOLE_DATABASE, CRSQL_TRACKED_EVENT_DDCP_MERGE, site_id],
                )?;
                tx.commit()
            })
            .await?;
        Ok(())
    }

    async fn dht_keypair(&self, name: &str) -> Result<DHTRecordDescriptor> {
        let dht = match load_local_keypair(&self.api, name).await? {
            Some((key, owner)) => {
                self.routing_context
                    .open_dht_record(key, Some(owner))
                    .await?
            }
            None => {
                let new_dht = self
                    .routing_context
                    .create_dht_record(DHTSchema::DFLT(DHTSchemaDFLT { o_cnt: 3 }), None)
                    .await?;
                store_local_keypair(
                    &self.api,
                    name,
                    new_dht.key(),
                    KeyPair::new(
                        new_dht.owner().to_owned(),
                        new_dht.owner_secret().unwrap().to_owned(),
                    ),
                )
                .await?;
                new_dht
            }
        };
        Ok(dht)
    }

    pub async fn cleanup(self) -> Result<()> {
        // Attempt to close DHT record
        if let Some(local_dht) = self.dht_keypair {
            if let Err(e) = self
                .routing_context
                .close_dht_record(local_dht.key().to_owned())
                .await
            {
                eprintln!("failed to close DHT record: {:?}", e);
            }
        }

        // Shut down Veilid node
        eprintln!("detach");
        self.api.detach().await?;
        eprintln!("shutting down");
        self.api.shutdown().await;
        eprintln!("shutdown");

        // Finalize cr-sqlite db
        self.conn
            .call(|c| {
                c.query_row("SELECT crsql_finalize()", [], |_row| Ok(()))?;
                Ok(())
            })
            .await?;
        eprintln!("crsql_finalized");

        Ok(())
    }

    pub async fn remote_add(&self, name: String, addr: String) -> Result<()> {
        let key = CryptoTyped::from_str(addr.as_str())?;
        store_remote_key(&self.api, &name, &key).await?;
        Ok(())
    }

    pub async fn remote_remove(&self, name: String) -> Result<()> {
        let db = &self.api.table_store()?.open("remote", 1).await?;
        db.delete(0, name.as_bytes()).await?;
        Ok(())
    }

    pub async fn remotes(&self) -> Result<Vec<(String, CryptoTyped<CryptoKey>)>> {
        let db = &self.api.table_store()?.open("remote", 1).await?;
        let keys = db.get_keys(0).await?;
        let mut result = vec![];
        for db_key in keys.iter() {
            let name = std::str::from_utf8(db_key.as_slice())?.to_owned();
            if let (Some(remote_key), _) = load_remote(&self.api, name.as_str()).await? {
                result.push((name, remote_key));
            }
        }
        Ok(result)
    }

    pub async fn serve(&mut self) -> Result<()> {
        // Create a private route and publish it
        let (route_id, route_blob) = self.api.new_private_route().await?;
        let local_dht = self.dht_keypair(LOCAL_KEYPAIR_NAME).await?;
        self.dht_keypair = Some(local_dht.clone());
        self.routing_context
            .set_dht_value(local_dht.key().to_owned(), SUBKEY_PRIVATE_ROUTE, route_blob)
            .await?;

        let (stop_sender, stop_receiver) = oneshot::channel::<()>();
        let tracker_handle = self.local_tracker(stop_receiver, local_dht.clone()).await?;

        println!("{}", local_dht.key().to_string());

        // Handle requests from peers
        loop {
            select! {
                res = self.updates.recv_async() => {
                    match res {
                        Ok(VeilidUpdate::AppCall(app_call)) => {
                            let request = match proto::decode_request_message(app_call.message()) {
                                Ok(r) => r,
                                Err(e) => {
                                    eprintln!("invalid request: {:?}", e);
                                    continue
                                }
                            };
                            match request {
                                Request::Status => {
                                    // TODO: spawn responder to unblock event loop
                                    let (site_id, db_version) = status(&self.conn).await?;
                                    let resp = proto::encode_response_message(Response::Status {
                                        site_id,
                                        db_version,
                                    })?;
                                    self.api.app_call_reply(app_call.id(), resp).await?;
                                }
                                Request::Changes { since_db_version } => {
                                    let (site_id, changes) = changes(&self.conn, since_db_version).await?;
                                    let resp = proto::encode_response_message(Response::Changes{
                                        site_id,
                                        changes,
                                    })?;
                                    self.api.app_call_reply(app_call.id(), resp).await?;
                                }
                            }
                        }
                        Ok(VeilidUpdate::Shutdown) => return Ok(()),
                        Ok(_) => {}
                        Err(e) => return Err(other_err(e)),
                    }
                }
                _ = signal::ctrl_c() => {
                    eprintln!("interrupt received");
                    stop_sender.send(()).map_err(|e| other_err(format!("{:?}", e)))?;
                    break
                }
            }
        }
        self.api.release_private_route(route_id)?;
        tracker_handle.await.map_err(other_err)??;
        Ok(())
    }

    /// Poll local db for changes in latest db version, update DHT.
    async fn local_tracker(
        &self,
        mut stop_receiver: tokio::sync::oneshot::Receiver<()>,
        local_dht: DHTRecordDescriptor,
    ) -> Result<JoinHandle<Result<()>>> {
        let conn = self.conn.clone();
        let routing_context = self.routing_context.clone();
        let key = local_dht.key().to_owned();
        let mut timer = tokio::time::interval(Duration::from_secs(60));

        let h = tokio::spawn(async move {
            let mut prev_db_version = -1;
            loop {
                select! {
                    _ = timer.tick() => {
                        let (site_id, db_version) = status(&conn).await?;
                        if db_version <= prev_db_version {
                            continue;
                        }

                        routing_context.set_dht_value(key, SUBKEY_SITE_ID, site_id).await?;
                        routing_context
                            .set_dht_value(key, SUBKEY_DB_VERSION, db_version.to_be_bytes().to_vec())
                            .await?;
                        prev_db_version = db_version;
                    }
                    _ = &mut stop_receiver => {
                        break;
                    }
                }
            }
            Ok(())
        });
        Ok(h)
    }
}

async fn status(conn: &Connection) -> Result<(Vec<u8>, i64)> {
    let (site_id, db_version) = conn
        .call(|c| {
            c.query_row(
                "
select crsql_site_id(), (select max(db_version)
from crsql_changes where site_id is null);",
                [],
                |row| Ok((row.get::<usize, Vec<u8>>(0)?, row.get::<usize, i64>(1)?)),
            )
        })
        .await?;
    Ok((site_id, db_version))
}

async fn changes(
    conn: &Connection,
    since_db_version: i64,
) -> Result<(Vec<u8>, Vec<proto::Change>)> {
    let (site_id, db_version) = status(conn).await?;
    if since_db_version >= db_version {
        return Ok((site_id, vec![]));
    }
    let changes = conn
        .call(move |c| {
            let mut stmt = c.prepare(
                "
select
    \"table\",
    pk,
    cid,
    cast(val as blob) as val,
    col_version,
    db_version,
    cl,
    seq
from crsql_changes
where db_version > ?
and site_id is null",
            )?;
            let changes_iter = stmt.query_map([since_db_version], |row| {
                Ok(proto::Change {
                    table: row.get::<usize, String>(0)?,
                    pk: row.get::<usize, Vec<u8>>(1)?,
                    cid: row.get::<usize, String>(2)?,
                    val: match row.get::<usize, Option<Vec<u8>>>(3)? {
                        Some(val) => val,
                        None => vec![],
                    },
                    col_version: row.get::<usize, i64>(4)?,
                    db_version: row.get::<usize, i64>(5)?,
                    cl: row.get::<usize, i64>(6)?,
                    seq: row.get::<usize, i64>(7)?,
                })
            })?;
            changes_iter.fold(Ok(vec![]), |acc, x| match acc {
                Ok(mut changes) => match x {
                    Ok(change) => {
                        changes.push(change);
                        Ok(changes)
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            })
        })
        .await?;
    Ok((site_id, changes))
}

async fn load_local_keypair(
    api: &VeilidAPI,
    name: &str,
) -> VeilidAPIResult<Option<(TypedKey, KeyPair)>> {
    let db = api.table_store()?.open("local", 2).await?;
    let key = db.load_json::<TypedKey>(0, name.as_bytes()).await?;
    let owner = db.load_json::<KeyPair>(1, name.as_bytes()).await?;
    Ok(match (key, owner) {
        (Some(k), Some(o)) => Some((k, o)),
        _ => None,
    })
}

async fn load_remote(
    api: &VeilidAPI,
    name: &str,
) -> VeilidAPIResult<(Option<TypedKey>, Option<Vec<u8>>)> {
    let db = api.table_store()?.open("remote", 2).await?;
    let key = db.load_json::<TypedKey>(0, name.as_bytes()).await?;
    let site_id = db.load_json::<Vec<u8>>(1, name.as_bytes()).await?;
    Ok((key, site_id))
}

async fn store_local_keypair(
    api: &VeilidAPI,
    name: &str,
    key: &TypedKey,
    owner: KeyPair,
) -> VeilidAPIResult<()> {
    let db = api.table_store()?.open("local", 2).await?;
    db.store_json(0, name.as_bytes(), key).await?;
    db.store_json(1, name.as_bytes(), &owner).await
}

async fn store_remote_key(api: &VeilidAPI, name: &str, key: &TypedKey) -> VeilidAPIResult<()> {
    let db = api.table_store()?.open("remote", 2).await?;
    db.store_json(0, name.as_bytes(), key).await
}

async fn store_remote_site_id(
    api: &VeilidAPI,
    name: &str,
    site_id: Vec<u8>,
) -> VeilidAPIResult<()> {
    let db = api.table_store()?.open("remote", 2).await?;
    db.store_json(1, name.as_bytes(), &site_id).await
}

pub fn other_err<T: ToString>(e: T) -> Error {
    Error::Other(e.to_string())
}
