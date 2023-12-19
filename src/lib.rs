use std::{sync::Arc, time::Duration};

use flume::{unbounded, Receiver, Sender};
use ident::{Conclave, Peer};
use rusqlite::{params, types::Value, OptionalExtension};
use tokio::{
    select, signal,
    task::{JoinHandle, JoinSet},
};
use tokio_rusqlite::Connection;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, trace, warn, Level};
use veilid_core::{
    CryptoKey, CryptoTyped, DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, FromStr, KeyPair,
    RouteId, RoutingContext, Sequencing, SharedSecret, Target, ValueSubkey, VeilidUpdate,
};

pub mod cli;
mod db;
mod error;
mod ident;
mod proto;
pub mod veilid_config;

use db::DB;
pub use error::{other_err, Error, Result};

pub struct DDCP {
    db: DB,
    routing_context: RoutingContext,
    conclave: Conclave,
    updates: Receiver<VeilidUpdate>,
}

static LOCAL_TRACKER_PERIOD: Duration = Duration::from_secs(60);
static REMOTE_TRACKER_PERIOD: Duration = Duration::from_secs(60);

impl DDCP {
    pub async fn new(db_path: Option<&str>, state_path: &str, ext_path: &str) -> Result<DDCP> {
        let db = DB::new(db_path, ext_path).await?;
        let (routing_context, updates) = DDCP::new_routing_context(state_path).await?;
        Ok(DDCP {
            db,
            conclave: Conclave::new(routing_context.clone()),
            routing_context,
            updates,
        })
    }

    async fn new_routing_context(
        state_path: &str,
    ) -> Result<(RoutingContext, Receiver<VeilidUpdate>)> {
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
            .routing_context()?
            .with_sequencing(Sequencing::EnsureOrdered)
            .with_default_safety()?;
        Ok((routing_context, updates))
    }

    #[instrument(skip(self), level = Level::INFO, ret, err)]
    pub async fn wait_for_network(&self) -> Result<()> {
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
                        break;
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

        // Set DHT state
        self.push().await?;
        Ok(())
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn push(&mut self) -> Result<(String, Vec<u8>, i64)> {
        let (site_id, db_version) = self.db.status();

        self.conclave
            .refresh(Some(Status {
                site_id,
                db_version,
            }))
            .await?;

        // Load or create DHT key
        let local_dht = self.ensure_dht_keypair(LOCAL_KEYPAIR_NAME).await?;
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

        Ok((
            self.conclave.sovereign().dht_key().to_string(),
            site_id,
            db_version,
        ))
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn pull(&mut self, name: &str) -> Result<(bool, i64)> {
        // Get latest status from DHT
        let peer = self.conclave.peer(name).ok_or(other_err("unknown peer"))?;
        let peer_status = peer
            .refresh(&self.routing_context)
            .await?
            .ok_or(other_err("peer missing status"))?;

        let tracked_version = self.db.tracked_peer_version(peer_status.site_id).await?;
        if peer_status.db_version <= tracked_version {
            return Ok((false, tracked_version));
        }

        debug!(
            db_version = peer_status.db_version,
            tracked_version, "pulling changes"
        );

        let resp = self.conclave.changes(peer, tracked_version).await?;
        if resp.site_id != &peer_status.site_id {
            return Err(other_err("mismatched site_id"));
        }

        let merge_version = self.db.merge(resp.site_id, resp.changes).await?;
        Ok((true, merge_version))
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn cleanup(self) -> Result<()> {
        // Release DHT resources
        self.conclave.close()?;

        // Release cr_sqlite resources
        self.db.close().await?;

        Ok(())
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn remote_add(&self, name: String, addr: String) -> Result<()> {
        let peer = Peer::new(&self.routing_context.api(), name.as_str(), addr.as_str()).await?;
        self.conclave.set_peer(peer).await?;
        Ok(())
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn remote_remove(&self, name: String) -> Result<()> {
        todo!(); // TODO: need to add Conclave::remove_peer
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn remotes(&self) -> Result<Vec<(String, CryptoTyped<CryptoKey>)>> {
        todo!(); // TODO: implement peer getters like this:
                 //self.conclave.peers().map(|peer| {(peer.name(), peer.dht_public_key.to_string())})
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn serve(&mut self) -> Result<()> {
        let mut run = true;

        while run {
            // Create a private route and publish it
            let (route_id, route_blob) = self.node.new_private_route().await?;
            let local_dht = self.ensure_dht_keypair(LOCAL_KEYPAIR_NAME).await?;
            self.dht_keypair = Some(local_dht.clone());
            self.node
                .set_dht_value(local_dht.key().to_owned(), SUBKEY_PRIVATE_ROUTE, route_blob)
                .await?;

            let cancel = CancellationToken::new();
            let local_tracker = self
                .local_tracker(cancel.clone(), local_dht.clone())
                .await?;
            let mut remote_tracker = self.remote_tracker(cancel.clone()).await?;

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
                        cancel.cancel();
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
            while let Some(res) = remote_tracker.join_next().await {
                if let Err(e) = res {
                    warn!(
                        err = format!("{:?}", e),
                        "Error shutting down remote tracker"
                    );
                }
            }
            if let Err(e) = local_tracker.await.map_err(other_err)? {
                warn!(err = format!("{:?}", e), "Error shutting own local tracker");
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
                let request = match proto::decode_message::<proto::request::Reader, proto::Request>(
                    app_call.message(),
                ) {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(err = format!("{:?}", e), "Invalid app_call request");
                        return Ok(false);
                    }
                };
                match request {
                    Request::Status => {
                        let (site_id, db_version) = status(&self.conn).await?;
                        let resp = proto::encode_message(&Response::Status {
                            site_id,
                            db_version,
                        })?;
                        let node = self.node.clone_box();
                        tokio::spawn(async move { node.app_call_reply(app_call.id(), resp).await });
                    }
                    Request::Changes { since_db_version } => {
                        let (site_id, changes) = changes(&self.conn, since_db_version).await?;
                        let resp = proto::encode_message(&Response::Changes { site_id, changes })?;
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
        cancel_token: CancellationToken,
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
                    _ = cancel_token.cancelled() => {
                        break;
                    }
                }
            }
            Ok(())
        });
        Ok(h)
    }

    /// Poll remotes for changes.
    #[instrument(skip(self, cancel_token), level = Level::DEBUG, err)]
    async fn remote_tracker(&self, cancel_token: CancellationToken) -> Result<JoinSet<Result<()>>> {
        let mut joins = JoinSet::new();
        let remotes = self.remotes().await?;
        for remote in remotes {
            let mut ddcp = Self::new_conn_node(self.conn.clone(), self.node.clone_box());
            let mut timer = tokio::time::interval(REMOTE_TRACKER_PERIOD);
            let remote_cancel = cancel_token.clone();

            let remote_name = remote.0.clone();
            joins.spawn(async move {
            loop {
                select! {
                    // TODO: use Veilid DHT watch when it's implemented
                    _ = timer.tick() => {
                        match ddcp.pull(&remote_name).await {
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
                    _ = remote_cancel.cancelled() => {
                        break;
                    }
                }
            }
            Ok(())
        });
        }
        Ok(joins)
    }
}

#[cfg(feature = "todo")]
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
    if let Err(e) = node.close_dht_record(dht.key().to_owned()).await {
        warn!(
            err = format!("{:?}", e),
            key = dht.key().to_string(),
            "failed to close dht record"
        )
    }

    if let (None, Some(site_id)) = (prior_site_id, &maybe_site_id) {
        node.store()
            .store_remote_site_id(name, site_id.data().to_owned())
            .await?;
    }

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

#[cfg(feature = "todo")]
#[instrument(skip(conn), level = Level::DEBUG, ret, err)]
pub async fn status(conn: &Connection) -> Result<NodeStatus> {
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
    Ok(NodeStatus {
        site_id,
        db_version,
        route: vec![],
    })
}

#[cfg(feature = "todo")]
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
