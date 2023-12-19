use std::{collections::HashMap, sync::Arc, time::Duration};

use flume::{unbounded, Receiver, Sender};
use tokio::{select, signal, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn, Level};
use veilid_core::{CryptoKey, CryptoTyped, RoutingContext, Sequencing, VeilidUpdate};

pub mod cli;
mod db;
mod error;
mod ident;
mod proto;
pub mod veilid_config;

use db::DB;
pub use error::{other_err, Error, Result};
use ident::{Conclave, Peer, Status};
use proto::codec::{
    ChangesResponse, Decodable, Envelope, NodeStatus, Request, Response, StatusResponse,
};

#[derive(Clone)]
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
            conclave: Conclave::new(routing_context.clone()).await?,
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
    pub async fn wait_for_network(&mut self) -> Result<()> {
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
        let (site_id, db_version) = self.db.status().await?;
        self.conclave
            .refresh(Status {
                site_id: site_id.clone(),
                db_version,
            })
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
        {
            let peer = self
                .conclave
                .peer_mut(name)
                .ok_or(other_err("unknown peer"))?;
            peer.refresh(&self.routing_context)
                .await?;
        }

        let peer = self.conclave.peer(name).ok_or(other_err("unknown peer"))?;
        let node_status = peer.node_status().ok_or(other_err("peer missing status"))?;
        self.pull_from(peer, &node_status).await
    }

    async fn pull_from(&self, peer: &Peer, peer_status: &NodeStatus) -> Result<(bool, i64)> {
        let tracked_version = self
            .db
            .tracked_peer_version(peer_status.site_id.clone())
            .await?;
        if peer_status.db_version <= tracked_version {
            return Ok((false, tracked_version));
        }

        debug!(
            db_version = peer_status.db_version,
            tracked_version, "pulling changes"
        );

        let resp = self.conclave.changes(peer, tracked_version).await?;
        if resp.site_id.as_slice() != peer_status.site_id.as_slice() {
            return Err(other_err("mismatched site_id"));
        }

        let merge_version = self.db.merge(resp.site_id, resp.changes).await?;
        Ok((true, merge_version))
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn cleanup(self) -> Result<()> {
        // Release DHT resources
        self.conclave.close().await?;

        // Shut down Veilid node
        self.routing_context.api().shutdown().await;

        // Release cr_sqlite resources
        self.db.close().await?;

        Ok(())
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn remote_add(&mut self, name: String, addr: String) -> Result<()> {
        let peer = Peer::new(&self.routing_context.api(), name.as_str(), addr.as_str()).await?;
        self.conclave.set_peer(peer).await?;
        Ok(())
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn remote_remove(&mut self, name: String) -> Result<bool> {
        self.conclave.remove_peer(name.as_str()).await
    }

    pub fn remotes(&self) -> Vec<(String, CryptoTyped<CryptoKey>)> {
        self.conclave
            .peers()
            .map(|peer| (peer.name(), peer.dht_key()))
            .collect()
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn serve(self) -> Result<()> {
        let token = CancellationToken::new();

        let puller = self.spawn_puller(token.clone());
        let server = self.spawn_server(token.clone());
        tokio::spawn(async move {
            signal::ctrl_c().await?;
            warn!("interrupt received");
            token.cancel();
            Ok::<(), Error>(())
        });

        tokio::join!(puller, server);
        Ok(())
    }

    async fn spawn_puller(&self, token: CancellationToken) -> JoinHandle<Result<()>> {
        let mut puller = self.clone();
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(REMOTE_TRACKER_PERIOD);
            loop {
                select! {
                    _ = timer.tick() => {
                        info!("refreshing peers");
                        if let Err(e) = puller.conclave.refresh_peers().await {
                            error!(err = format!("{:?}", e), "refresh failed");
                            continue
                        }

                        for peer in puller.conclave.peers() {
                            info!(name = peer.name(), "pulling from peer");
                            let peer_status = match peer.node_status() {
                                    Some(status) => status,
                                    None => {
                                        error!("peer missing status");
                                        continue
                                    }
                                };
                            if let Err(e) = puller.pull_from(peer, &peer_status).await {
                                error!(err = format!("{:?}", e), name = peer.name(), "pull failed");
                            } else {
                                info!(name = peer.name(), "pull ok");
                            }
                        }
                    }
                    _ = token.cancelled() => {
                        return Ok(());
                    }
                }
            }
        })
    }

    async fn spawn_server(&self, token: CancellationToken) -> JoinHandle<Result<()>> {
        let mut server = self.clone();
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(LOCAL_TRACKER_PERIOD);
            let peer_by_key = HashMap::from_iter(
                server
                    .conclave
                    .peers()
                    .map(|peer| (peer.dht_key().to_string(), peer.clone())),
            );
            loop {
                select! {
                    _ = timer.tick() => {
                        // TODO: spawn this? how long does it block the loop?
                        if let Err(e) = server.push().await {
                            error!(err = format!("{:?}", e), "failed to push status");
                        }
                    }
                    res = server.updates.recv_async() => {
                        match res {
                            Ok(update) => {
                                if let Err(e) = server.handle_update(&peer_by_key, update).await {
                                    error!(err = format!("{:?}", e), "failed to handle update");
                                }
                            }
                            Err(e) => return Err(other_err(e)),
                        }
                    }
                    _ = token.cancelled() => {
                        return server.cleanup().await;
                    }
                }
            }
        })
    }

    async fn handle_update(
        &mut self,
        peer_by_sender: &HashMap<String, Peer>,
        update: VeilidUpdate,
    ) -> Result<()> {
        match update {
            VeilidUpdate::AppCall(app_call) => {
                let envelope = Envelope::decode(app_call.message())?;
                let peer = match peer_by_sender.get(&envelope.sender) {
                    Some(peer) => peer.clone(),
                    None => {
                        warn!(sender = envelope.sender, "unknown peer");
                        return Ok(());
                    }
                };

                // spawn a responder, don't block the event loop
                let responder = self.clone();
                tokio::spawn(async move {
                    let crypto = responder.conclave.crypto(&peer)?;
                    let request = crypto.decode::<Request>(app_call.message())?;
                    let resp = match request {
                        Request::Status => {
                            let (site_id, db_version) = responder.db.status().await?;
                            crypto.encode(Response::Status(StatusResponse {
                                site_id,
                                db_version,
                            }))?
                        }
                        Request::Changes { since_db_version } => {
                            let (site_id, changes) = responder.db.changes(since_db_version).await?;
                            crypto
                                .encode(Response::Changes(ChangesResponse { site_id, changes }))?
                        }
                    };
                    responder
                        .routing_context
                        .api()
                        .app_call_reply(app_call.id(), resp)
                        .await?;
                    Ok::<(), Error>(())
                }); // TODO: instrument
                Ok(())
            }
            VeilidUpdate::Shutdown => Err(Error::VeilidAPI(veilid_core::VeilidAPIError::Shutdown)),
            VeilidUpdate::RouteChange(_) => {
                self.conclave
                    .sovereign_mut()
                    .release_route(&self.routing_context)?;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
