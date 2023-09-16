use std::{io, path::PathBuf, sync::Arc};

use clap::Parser;
use flume::{unbounded, Receiver, Sender};
use tokio_rusqlite::Connection;
use veilid_core::{
    DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, KeyPair, RoutingContext, Sequencing, TypedKey,
    VeilidAPI, VeilidAPIError, VeilidAPIResult, VeilidUpdate,
};

mod cli;
mod veilid_config;

use cli::{Cli, Commands};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("db error: {0}")]
    DB(#[from] tokio_rusqlite::Error),
    #[error("io error: {0}")]
    IO(#[from] io::Error),
    #[error("veilid api error: {0}")]
    VeilidAPI(#[from] VeilidAPIError),
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

    app.wait_for_network().await?;

    // magic gonna happen here
    let result = match cli.commands {
        Commands::Init => app.init().await,
        Commands::Push => app.push().await,
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

const LOCAL_PEER_NAME: &'static str = "__local";

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
        let local_dht = self.dht_key(LOCAL_PEER_NAME).await?;
        println!("{}", local_dht.key().to_string());
        Ok(())
    }

    pub async fn push(&mut self) -> Result<()> {
        // Load or create DHT key
        let local_dht = self.dht_key(LOCAL_PEER_NAME).await?;
        println!("{}", local_dht.key().to_string());

        let key = local_dht.key().to_owned();

        // Push current db state
        let (site_id, db_version) = self.conn.call(|c| {
            c.query_row("
            select crsql_site_id(), (select max(db_version) from crsql_changes where site_id is null);
            ", [], |row| {
                Ok((
                    row.get::<usize, Vec<u8>>(0)?,
                    row.get::<usize, i64>(1)?,
                ))
            })
        }).await?;
        self.routing_context.set_dht_value(key, 0, site_id).await?;
        self.routing_context
            .set_dht_value(key, 1, db_version.to_be_bytes().to_vec())
            .await?;

        Ok(())
    }

    async fn dht_key(&self, name: &str) -> Result<DHTRecordDescriptor> {
        let dht = match load_dht_key(&self.api, name).await? {
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
                store_dht_key(
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
}

async fn load_dht_key(api: &VeilidAPI, name: &str) -> VeilidAPIResult<Option<(TypedKey, KeyPair)>> {
    let db = api.table_store()?.open("velouria", 2).await?;
    let key = db.load_json::<TypedKey>(0, name.as_bytes()).await?;
    let owner = db.load_json::<KeyPair>(1, name.as_bytes()).await?;
    Ok(match (key, owner) {
        (Some(k), Some(o)) => Some((k, o)),
        _ => None,
    })
}

async fn store_dht_key(
    api: &VeilidAPI,
    name: &str,
    key: &TypedKey,
    owner: KeyPair,
) -> VeilidAPIResult<()> {
    let db = api.table_store()?.open("velouria", 2).await?;
    db.store_json(0, name.as_bytes(), key).await?;
    db.store_json(1, name.as_bytes(), &owner).await
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
