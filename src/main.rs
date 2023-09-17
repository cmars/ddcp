use std::{f32::consts::E, io, path::PathBuf, sync::Arc, time::Duration};

use clap::Parser;
use flume::{unbounded, Receiver, Sender};
use tokio::task::JoinHandle;
use tokio_rusqlite::Connection;
use veilid_core::{
    CryptoTyped, DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, FromStr, KeyPair, RoutingContext,
    Sequencing, TypedKey, VeilidAPI, VeilidAPIError, VeilidAPIResult, VeilidUpdate,
};

mod cli;
mod proto;
mod veilid_config;

use cli::{Cli, Commands, RemoteArgs, RemoteCommands};
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
    #[error("other: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[tokio::main]
async fn main() {
    run().await.expect("ok");
}

async fn run() -> Result<()> {
    let cli = cli::Cli::parse();

    let mut app = Velouria::new(&cli).await?;

    if cli.needs_network() {
        app.wait_for_network().await?;
    }

    let result = match cli.commands {
        Commands::Init => app.init().await,
        Commands::Push => app.push().await,
        Commands::Remote(RemoteArgs {
            commands: RemoteCommands::Add { name, addr },
        }) => app.remote_add(name, addr).await,
        Commands::Remote(RemoteArgs {
            commands: RemoteCommands::Remove { name },
        }) => app.remote_remove(name).await,
        Commands::Remote(RemoteArgs {
            commands: RemoteCommands::List,
        }) => app.remote_list().await,
        Commands::Serve => app.serve().await,
        _ => Err(Error::Other("unsupported command".to_string())),
    };

    app.cleanup().await?;
    result
}

pub struct Velouria {
    conn: Connection,
    api: VeilidAPI,
    updates: Receiver<VeilidUpdate>,
    routing_context: RoutingContext,
}

const LOCAL_KEYPAIR_NAME: &'static str = "__local";

impl Velouria {
    pub async fn new(cli: &Cli) -> Result<Velouria> {
        let conn = Velouria::new_connection(cli).await?;
        let (api, updates) = Velouria::new_veilid_node(cli).await?;
        let routing_context = api
            .routing_context()
            .with_sequencing(Sequencing::EnsureOrdered)
            .with_privacy()?;
        Ok(Velouria {
            conn,
            api,
            updates,
            routing_context,
        })
    }

    async fn new_connection(cli: &Cli) -> Result<Connection> {
        let conn = Connection::open(cli.path("velouria.db")).await?;
        let ext_path = ext_path()?;
        conn.call(move |c| {
            unsafe {
                c.load_extension_enable()?;
                let r = c.load_extension(ext_path.as_str(), Some("sqlite3_crsqlite_init"))?;
                c.load_extension_disable()?;
                r
            };
            Ok(())
        })
        .await?;
        Ok(conn)
    }

    async fn new_veilid_node(cli: &Cli) -> Result<(VeilidAPI, Receiver<VeilidUpdate>)> {
        // Veilid API state channel
        let (node_sender, updates): (
            Sender<veilid_core::VeilidUpdate>,
            Receiver<veilid_core::VeilidUpdate>,
        ) = unbounded();

        // Start up Veilid core
        let update_callback = Arc::new(move |change: veilid_core::VeilidUpdate| {
            let _ = node_sender.send(change);
        });
        let app_dir = Arc::new(cli.app_dir());
        let config_callback =
            Arc::new(move |key| veilid_config::callback(app_dir.to_string(), key));
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
        // Load or create DHT key
        let local_dht = self.dht_key(LOCAL_KEYPAIR_NAME).await?;
        println!("{}", local_dht.key().to_string());
        Ok(())
    }

    pub async fn push(&mut self) -> Result<()> {
        // Load or create DHT key
        let local_dht = self.dht_key(LOCAL_KEYPAIR_NAME).await?;
        println!("{}", local_dht.key().to_string());

        let key = local_dht.key().to_owned();

        let (site_id, db_version) = status(&self.conn).await?;

        // Push current db state
        self.routing_context.set_dht_value(key, 0, site_id).await?;
        self.routing_context
            .set_dht_value(key, 1, db_version.to_be_bytes().to_vec())
            .await?;

        Ok(())
    }

    async fn dht_key(&self, name: &str) -> Result<DHTRecordDescriptor> {
        let dht = match load_dht_keypair(&self.api, name).await? {
            Some((key, owner)) => {
                self.routing_context
                    .open_dht_record(key, Some(owner))
                    .await?
            }
            None => {
                let new_dht = self
                    .routing_context
                    .create_dht_record(DHTSchema::DFLT(DHTSchemaDFLT { o_cnt: 2 }), None)
                    .await?;
                store_dht_keypair(
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

    async fn remote_add(&self, name: String, addr: String) -> Result<()> {
        let key = CryptoTyped::from_str(addr.as_str())?;
        store_dht_key(&self.api, &name, &key).await?;
        Ok(())
    }

    async fn remote_remove(&self, name: String) -> Result<()> {
        let db = &self.api.table_store()?.open("remote", 1).await?;
        db.delete(0, name.as_bytes()).await?;
        Ok(())
    }

    async fn remote_list(&self) -> std::result::Result<(), Error> {
        let db = &self.api.table_store()?.open("remote", 1).await?;
        let keys = db.get_keys(0).await?;
        for key in keys.iter() {
            let name = std::str::from_utf8(key.as_slice()).expect("valid utf-8");
            if let Some(remote_key) = load_dht_key(&self.api, name).await? {
                println!("{} {}", name, remote_key.to_string());
            }
        }
        Ok(())
    }

    async fn serve(&self) -> Result<()> {
        let _ = self.local_tracker();
        loop {
            match self.updates.recv_async().await {
                Ok(VeilidUpdate::AppCall(app_call)) => {
                    let request = proto::decode_request_message(app_call.message())?;
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
                            let changes = changes(&self.conn, since_db_version)?;
                            let resp = proto::encode_response_message(Response::Changes(changes))?;
                            self.api.app_call_reply(app_call.id(), resp)?;
                        }
                    }
                }
                Ok(VeilidUpdate::Shutdown) => return Ok(()),
                Ok(_) => {}
                Err(e) => return Err(other_err(e)),
            }
        }
    }

    async fn local_tracker(&self) -> Result<JoinHandle<Result<()>>> {
        let conn = self.conn.clone();
        let routing_context = self.routing_context.clone();
        let local_dht = self.dht_key(LOCAL_KEYPAIR_NAME).await?;
        let key = local_dht.key().to_owned();

        let mut timer = tokio::time::interval(Duration::from_secs(60));
        let h = tokio::spawn(async move {
            let mut prev_db_version = -1;
            loop {
                // TODO: select! for shutdown signal
                timer.tick().await;
                let (site_id, db_version) = status(&conn).await?;
                if db_version <= prev_db_version {
                    continue;
                }

                routing_context.set_dht_value(key, 0, site_id).await?;
                routing_context
                    .set_dht_value(key, 1, db_version.to_be_bytes().to_vec())
                    .await?;
                prev_db_version = db_version;
            }
        });
        Ok(h)
    }
}

async fn status(conn: &Connection) -> Result<(Vec<u8>, i64)> {
    let (site_id, db_version) = conn.call(|c| {
            c.query_row("
            select crsql_site_id(), (select max(db_version) from crsql_changes where site_id is null);
            ", [], |row| {
                Ok((
                    row.get::<usize, Vec<u8>>(0)?,
                    row.get::<usize, i64>(1)?,
                ))
            })
        }).await?;
    Ok((site_id, db_version))
}

async fn changes(conn: &Connection, since_db_version: i64) -> Result<Vec<proto::Change>> {
    let changes = conn
        .call(move |c| {
            let mut stmt = c.prepare(
                "
            select
                table,
                pk,
                cid,
                val,
                col_version,
                db_version,
                site_id,
                cl,
                seq
            from crsql_changes
            where db_version > ?
            and site_id is null
            ",
            )?;
            let changes_iter = stmt.query_map([since_db_version], |row| {
                Ok(proto::Change {
                    table: row.get::<usize, String>(0)?,
                    pk: row.get::<usize, String>(1)?,
                    cid: row.get::<usize, String>(2)?,
                    val: row.get::<usize, Vec<u8>>(3)?,
                    col_version: row.get::<usize, i64>(4)?,
                    db_version: row.get::<usize, i64>(5)?,
                    site_id: vec![],
                    cl: row.get::<usize, i64>(7)?,
                    seq: row.get::<usize, i64>(8)?,
                })
            })?;
            changes_iter.fold(Ok(vec![]), |acc, x| {
                match acc {
                    Ok(mut changes) => {
                        match x {
                            Ok(change) => {
                                changes.push(change);
                                Ok(changes)
                            }
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                }
            })
        })
        .await?;
    Ok(changes)
}

async fn load_dht_keypair(
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

async fn load_dht_key(api: &VeilidAPI, name: &str) -> VeilidAPIResult<Option<TypedKey>> {
    let db = api.table_store()?.open("remote", 1).await?;
    let key = db.load_json::<TypedKey>(0, name.as_bytes()).await?;
    Ok(key)
}

async fn store_dht_keypair(
    api: &VeilidAPI,
    name: &str,
    key: &TypedKey,
    owner: KeyPair,
) -> VeilidAPIResult<()> {
    let db = api.table_store()?.open("local", 2).await?;
    db.store_json(0, name.as_bytes(), key).await?;
    db.store_json(1, name.as_bytes(), &owner).await
}

async fn store_dht_key(api: &VeilidAPI, name: &str, key: &TypedKey) -> VeilidAPIResult<()> {
    let db = api.table_store()?.open("remote", 1).await?;
    db.store_json(0, name.as_bytes(), key).await
}

fn other_err<T: ToString>(e: T) -> Error {
    Error::Other(e.to_string())
}

fn ext_path() -> Result<String> {
    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent().expect("executable has a parent directory");
    Ok(String::from(
        exe_dir
            .join(PathBuf::from("crsqlite"))
            .as_os_str()
            .to_str()
            .expect("valid path string"),
    ))
}
