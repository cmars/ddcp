use std::{sync::Arc, time::Duration};

use flume::{unbounded, Receiver, Sender};
use rand::{rngs::StdRng, Rng, SeedableRng};
use rusqlite::{params, types::Value, OptionalExtension};
use tokio::{select, signal, sync::oneshot, task::JoinHandle};
use tokio_rusqlite::Connection;
use tracing::{debug, error, info, instrument, trace, warn, Level};
use veilid_core::{
    CryptoKey, CryptoTyped, DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, FromStr, KeyPair,
    RouteId, Sequencing, Target, ValueSubkey, VeilidUpdate,
};

pub mod cli;
mod error;
mod node;
mod proto;
mod store;
pub mod veilid_config;

pub use error::{other_err, Error, Result};
pub use node::{Node, VeilidNode};
use proto::{Request, Response};

pub struct DDCP {
    conn: Connection,
    node: Box<dyn Node>,
    dht_keypair: Option<DHTRecordDescriptor>,
}

pub(crate) const LOCAL_KEYPAIR_NAME: &'static str = "__local";

pub(crate) const DHT_SUBKEY_COUNT: u16 = 3;
pub(crate) const SUBKEY_SITE_ID: ValueSubkey = 0;
pub(crate) const SUBKEY_DB_VERSION: ValueSubkey = 1;
pub(crate) const SUBKEY_PRIVATE_ROUTE: ValueSubkey = 2;

pub(crate) const CRSQL_TRACKED_TAG_WHOLE_DATABASE: i32 = 0;
pub(crate) const CRSQL_TRACKED_EVENT_RECEIVE: i32 = 0;

static LOCAL_TRACKER_PERIOD: Duration = Duration::from_secs(60);
static REMOTE_TRACKER_PERIOD: Duration = Duration::from_secs(60);

impl DDCP {
    pub async fn new(db_path: Option<&str>, state_path: &str, ext_path: &str) -> Result<DDCP> {
        let conn = DDCP::new_connection(db_path, ext_path).await?;
        let node = DDCP::new_veilid_node(state_path).await?;
        Ok(DDCP {
            conn,
            node,
            dht_keypair: None,
        })
    }

    pub(crate) fn new_conn_node(conn: Connection, node: Box<dyn Node>) -> DDCP {
        DDCP {
            conn,
            node,
            dht_keypair: None,
        }
    }

    pub(crate) async fn new_connection(
        db_path: Option<&str>,
        ext_path: &str,
    ) -> Result<Connection> {
        let conn = match db_path {
            Some(path) => Connection::open(path).await,
            None => Connection::open_in_memory().await,
        }?;
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

    async fn new_veilid_node(state_path: &str) -> Result<Box<dyn Node>> {
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

        let routing_context = api
            .routing_context()
            .with_sequencing(Sequencing::EnsureOrdered)
            .with_privacy()?;

        Ok(VeilidNode::new(routing_context, updates))
    }

    #[instrument(skip(self), level = Level::INFO, ret, err)]
    pub async fn wait_for_network(&self) -> Result<()> {
        self.node.wait_for_network().await
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn init(&mut self) -> Result<String> {
        // Load or create DHT key
        let local_dht = self.dht_keypair(LOCAL_KEYPAIR_NAME).await?;
        let addr = local_dht.key().to_string();
        self.dht_keypair = Some(local_dht);
        Ok(addr)
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn push(&mut self) -> Result<(String, Vec<u8>, i64)> {
        // Load or create DHT key
        let local_dht = self.dht_keypair(LOCAL_KEYPAIR_NAME).await?;
        let addr = local_dht.key().to_string();
        self.dht_keypair = Some(local_dht.clone());

        let key = local_dht.key().to_owned();

        let (site_id, db_version) = status(&self.conn).await?;

        // Push current db state
        self.node
            .set_dht_value(key, SUBKEY_SITE_ID, site_id.clone())
            .await?;
        self.node
            .set_dht_value(key, SUBKEY_DB_VERSION, db_version.to_be_bytes().to_vec())
            .await?;

        Ok((addr, site_id, db_version))
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn pull(&mut self, name: &str) -> Result<(bool, i64)> {
        // Get latest status from DHT
        let (site_id, db_version, route_blob) = remote_status(self.node.clone_box(), name).await?;

        // Do we need to fetch newer versions?
        let tracked_version = match self
            .conn
            .call(move |conn| {
                conn.query_row(
                    "
select max(coalesce(version, 0)) from crsql_tracked_peers
where site_id = ? and event = ?",
                    params![site_id, CRSQL_TRACKED_EVENT_RECEIVE],
                    |row| row.get::<usize, Option<i64>>(0),
                )
                .optional()
            })
            .await?
        {
            Some(Some(tracked_version)) => {
                if db_version <= tracked_version {
                    debug!("remote database {} is up to date", name);
                    return Ok((false, db_version));
                }
                tracked_version
            }
            _ => 0,
        };

        debug!(db_version, tracked_version, "pulling changes");

        let route_id = self.node.import_remote_private_route(route_blob)?;
        let msg_bytes = self
            .node
            .app_call(
                Target::PrivateRoute(route_id),
                proto::encode_request_message(Request::Changes {
                    since_db_version: tracked_version,
                })?,
            )
            .await?;
        let resp = proto::decode_response_message(msg_bytes.as_slice())?;

        if let Response::Changes { site_id, changes } = resp {
            self.merge(site_id.clone(), changes).await?;
        } else {
            return Err(Error::Other(
                "Invalid response to changes request".to_string(),
            ));
        }

        if let Err(e) = self.node.release_private_route(route_id) {
            warn!(
                route_id = route_id.to_string(),
                err = format!("{:?}", e),
                "Failed to release private route"
            );
        }

        Ok((true, db_version))
    }

    #[instrument(skip(self, changes), level = Level::DEBUG, ret, err)]
    pub(crate) async fn merge(
        &mut self,
        site_id: Vec<u8>,
        changes: Vec<proto::Change>,
    ) -> Result<i64> {
        let result = self
            .conn
            .call(move |conn| {
                let tx = conn.transaction()?;
                let mut ins_changes = tx.prepare(
                    "
insert into crsql_changes (
    \"table\", pk, cid, val, col_version, db_version, site_id, cl, seq)
values (?, ?, ?, ?, ?, ?, ?, ?, ?);",
                )?;
                let mut max_db_version = 0;
                for i in 0..changes.len() {
                    if changes[i].db_version > max_db_version {
                        max_db_version = changes[i].db_version;
                    }
                    ins_changes.execute(params![
                        changes[i].table,
                        changes[i].pk,
                        changes[i].cid,
                        changes[i].val,
                        changes[i].col_version,
                        changes[i].db_version,
                        &site_id,
                        changes[i].cl,
                        changes[i].seq,
                    ])?;
                    trace!("merge change {:?} {:?}", changes[i], site_id);
                }
                trace!(site_id = format!("{:?}", site_id), max_db_version);
                tx.execute(
                    "
insert into crsql_tracked_peers (site_id, version, tag, event)
values (?, ?, ?, ?)
on conflict do update set version = max(version, excluded.version)",
                    params![
                        site_id,
                        max_db_version,
                        CRSQL_TRACKED_TAG_WHOLE_DATABASE,
                        CRSQL_TRACKED_EVENT_RECEIVE
                    ],
                )?;
                drop(ins_changes);
                tx.commit()?;
                Ok(max_db_version)
            })
            .await?;
        Ok(result)
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub(crate) async fn dht_keypair(&self, name: &str) -> Result<DHTRecordDescriptor> {
        let dht = match self.node.store().load_local_keypair(name).await? {
            Some((key, owner)) => self.node.open_dht_record(key, Some(owner)).await?,
            None => {
                let new_dht = self
                    .node
                    .create_dht_record(
                        DHTSchema::DFLT(DHTSchemaDFLT {
                            o_cnt: DHT_SUBKEY_COUNT,
                        }),
                        None,
                    )
                    .await?;
                self.node
                    .store()
                    .store_local_keypair(
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

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn cleanup(self) -> Result<()> {
        let remotes = self.remotes().await?;
        for (name, _) in remotes.iter() {
            if let (Some(remote_dht), _) = self.node.store().load_remote(name).await? {
                let _ = self.node.close_dht_record(remote_dht).await;
            }
        }

        // Shut down Veilid node
        self.node.shutdown(self.dht_keypair).await?;

        // Finalize cr-sqlite db
        self.conn
            .call(|c| {
                c.query_row("SELECT crsql_finalize()", [], |_row| Ok(()))?;
                Ok(())
            })
            .await?;
        debug!("crsql_finalized");

        Ok(())
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn remote_add(&self, name: String, addr: String) -> Result<()> {
        let key = CryptoTyped::from_str(addr.as_str())?;
        self.node.store().store_remote_key(&name, &key).await?;
        Ok(())
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn remote_remove(&self, name: String) -> Result<()> {
        self.node.store().remove_remote(name).await?;
        Ok(())
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn remotes(&self) -> Result<Vec<(String, CryptoTyped<CryptoKey>)>> {
        let result = self.node.store().remotes().await?;
        Ok(result)
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn serve(&mut self) -> Result<()> {
        let mut run = true;

        while run {
            // Create a private route and publish it
            let (route_id, route_blob) = self.node.new_private_route().await?;
            let local_dht = self.dht_keypair(LOCAL_KEYPAIR_NAME).await?;
            self.dht_keypair = Some(local_dht.clone());
            self.node
                .set_dht_value(local_dht.key().to_owned(), SUBKEY_PRIVATE_ROUTE, route_blob)
                .await?;

            let (stop_local_sender, stop_local_receiver) = oneshot::channel::<()>();
            let local_tracker = self
                .local_tracker(stop_local_receiver, local_dht.clone())
                .await?;
            let (stop_remote_sender, stop_remote_receiver) = oneshot::channel::<()>();
            let remote_tracker = self.remote_tracker(stop_remote_receiver).await?;

            info!(key = local_dht.key().to_string(), "Serving database at DHT");

            // Handle requests from peers
            loop {
                select! {
                    res = self.node.recv_updates_async() => {
                        match res {
                            Ok(update) => {
                                let restart = self.handle_update(update, &route_id).await?;
                                if restart {
                                    info!("Restarting");
                                    break
                                }
                            }
                            Err(e) => return Err(other_err(e)),
                        }
                    }
                    _ = signal::ctrl_c() => {
                        warn!("Interrupt received");
                        if let Err(e) = stop_local_sender.send(()).map_err(|e| other_err(format!("{:?}", e))) {
                            warn!(err = format!("{:?}", e), "Failed to send shutdown signal to local tracker");
                        }
                        if let Err(e) = stop_remote_sender.send(()).map_err(|e| other_err(format!("{:?}", e))) {
                            warn!(err = format!("{:?}", e), "Failed to send shutdown signal to remote tracker");
                        }
                        run = false;
                        break
                    }
                }
            }
            if let Err(e) = self.node.release_private_route(route_id) {
                warn!(
                    route_id = route_id.to_string(),
                    err = format!("{:?}", e),
                    "Failed to release private route"
                );
            }
            if let Err(e) = local_tracker.await.map_err(other_err)? {
                warn!(
                    err = format!("{:?}", e),
                    "Error shutting down local tracker"
                );
            }
            if let Err(e) = remote_tracker.await.map_err(other_err)? {
                warn!(
                    err = format!("{:?}", e),
                    "Error shutting down remote tracker"
                );
            }
        }
        Ok(())
    }

    pub(crate) async fn handle_update(
        &mut self,
        update: VeilidUpdate,
        current_route: &RouteId,
    ) -> Result<bool> {
        match update {
            VeilidUpdate::AppCall(app_call) => {
                let request = match proto::decode_request_message(app_call.message()) {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(err = format!("{:?}", e), "Invalid app_call request");
                        return Ok(false);
                    }
                };
                match request {
                    Request::Status => {
                        let (site_id, db_version) = status(&self.conn).await?;
                        let resp = proto::encode_response_message(Response::Status {
                            site_id,
                            db_version,
                        })?;
                        let node = self.node.clone_box();
                        tokio::spawn(async move { node.app_call_reply(app_call.id(), resp).await });
                    }
                    Request::Changes { since_db_version } => {
                        let (site_id, changes) = changes(&self.conn, since_db_version).await?;
                        let resp =
                            proto::encode_response_message(Response::Changes { site_id, changes })?;
                        let node = self.node.clone_box();
                        tokio::spawn(async move { node.app_call_reply(app_call.id(), resp).await });
                    }
                }
            }
            VeilidUpdate::Shutdown => {
                return Err(Error::VeilidAPI(veilid_core::VeilidAPIError::Shutdown))
            }
            VeilidUpdate::RouteChange(route_change) => {
                for dead_route in route_change.dead_routes {
                    if dead_route == *current_route {
                        warn!(route = current_route.to_string(), "Current route is dead");
                        return Ok(true);
                    }
                }
            }
            _ => return Ok(false),
        }
        Ok(false)
    }

    /// Poll local db for changes in latest db version, update DHT.
    #[instrument(skip(self), level = Level::DEBUG, err)]
    async fn local_tracker(
        &self,
        mut stop_receiver: tokio::sync::oneshot::Receiver<()>,
        local_dht: DHTRecordDescriptor,
    ) -> Result<JoinHandle<Result<()>>> {
        let conn = self.conn.clone();
        let mut node = self.node.clone_box();
        let key = local_dht.key().to_owned();
        let mut timer = tokio::time::interval(LOCAL_TRACKER_PERIOD);

        let h = tokio::spawn(async move {
            let mut prev_db_version = -1;
            loop {
                select! {
                    // TODO: watch db for changes (filesystem changes, sqlite magic, control socket)
                    _ = timer.tick() => {
                        let (site_id, db_version) = status(&conn).await?;
                        if db_version <= prev_db_version {
                            continue;
                        }

                        node.set_dht_value(key, SUBKEY_SITE_ID, site_id).await?;
                        node
                            .set_dht_value(key, SUBKEY_DB_VERSION, db_version.to_be_bytes().to_vec())
                            .await?;
                        info!(db_version, key = key.to_string(), "Database changed, updated status");
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

    /// Poll remotes for changes.
    #[instrument(skip(self), level = Level::DEBUG, err)]
    async fn remote_tracker(
        &self,
        mut stop_receiver: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<JoinHandle<Result<()>>> {
        let mut ddcp = Self::new_conn_node(self.conn.clone(), self.node.clone_box());
        let mut timer = tokio::time::interval(REMOTE_TRACKER_PERIOD);

        let h = tokio::spawn(async move {
            let remotes = ddcp.remotes().await?;
            let mut rng: StdRng = SeedableRng::from_entropy();
            loop {
                select! {
                    // TODO: use Veilid DHT watch when it's implemented
                    _ = timer.tick() => {
                        let remote_index = rng.gen_range(0..remotes.len());
                        let remote_name = &remotes[remote_index].0;
                        match ddcp.pull(remote_name).await {
                            Ok((updated, db_version)) => {
                                if updated {
                                    info!(remote_name, db_version, "Pulled changes from remote database");
                                } else {
                                    debug!(remote_name, updated, db_version, "no changes");
                                }
                            }
                            Err(e) => {
                                error!("Failed to pull from {}: {:?}", remote_name, e);
                            }
                        }
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

#[instrument(skip(node), level = Level::DEBUG, err)]
pub(crate) async fn remote_status(
    node: Box<dyn Node>,
    name: &str,
) -> Result<(Vec<u8>, i64, Vec<u8>)> {
    let (prior_dht_key, prior_site_id) = node.store().load_remote(name).await?;
    let dht = match prior_dht_key {
        Some(key) => node.open_dht_record(key, None).await?,
        None => return Err(other_err(format!("remote {} not found", name))),
    };
    let maybe_site_id = node
        .get_dht_value(dht.key().to_owned(), SUBKEY_SITE_ID, true)
        .await?;
    let maybe_db_version = node
        .get_dht_value(dht.key().to_owned(), SUBKEY_DB_VERSION, true)
        .await?;
    let maybe_route_blob = node
        .get_dht_value(dht.key().to_owned(), SUBKEY_PRIVATE_ROUTE, true)
        .await?;
    let _ = node.close_dht_record(dht.key().to_owned()).await?;

    match (prior_site_id, &maybe_site_id) {
        (None, Some(site_id)) => {
            node.store()
                .store_remote_site_id(name, site_id.data().to_owned())
                .await?;
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

#[instrument(skip(conn), level = Level::DEBUG, ret, err)]
pub async fn status(conn: &Connection) -> Result<(Vec<u8>, i64)> {
    let (site_id, db_version) = conn
        .call(|c| {
            c.query_row(
                "
select crsql_site_id(), coalesce((select max(db_version) from crsql_changes where site_id is null), 0);",
                [],
                |row| Ok((row.get::<usize, Vec<u8>>(0)?, row.get::<usize, i64>(1)?)),
            )
        })
        .await?;
    Ok((site_id, db_version))
}

#[instrument(skip(conn), level = Level::DEBUG)]
pub async fn changes(
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
    val,
    col_version,
    db_version,
    cl,
    seq
from crsql_changes
where db_version > ?
and site_id is null",
            )?;
            let mut result = vec![];
            let mut rows = stmt.query([since_db_version])?;
            while let Some(row) = rows.next()? {
                let change = proto::Change {
                    table: row.get::<usize, String>(0)?,
                    pk: row.get::<usize, Vec<u8>>(1)?,
                    cid: row.get::<usize, String>(2)?,
                    val: row.get::<usize, Value>(3)?,
                    col_version: row.get::<usize, i64>(4)?,
                    db_version: row.get::<usize, i64>(5)?,
                    cl: row.get::<usize, i64>(6)?,
                    seq: row.get::<usize, i64>(7)?,
                };
                result.push(change);
            }
            Ok(result)
        })
        .await?;
    Ok((site_id, changes))
}

#[cfg(test)]
mod tests;
