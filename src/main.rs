use std::{io, path::PathBuf, sync::Arc};

use flume::{unbounded, Receiver, Sender};
use rusqlite::{Connection, OptionalExtension};
use veilid_core::{VeilidAPIError, VeilidUpdate};

mod config;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("db error: {0}")]
    DB(#[from] rusqlite::Error),
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
    let conn = Connection::open(":memory:")?;
    let ext_path = ext_path()?;
    unsafe {
        conn.load_extension_enable()?;
        let r = conn.load_extension(ext_path.as_str(), Some("sqlite3_crsqlite_init"))?;
        conn.load_extension_disable()?;
        r
    };

    let xdg_dirs = xdg::BaseDirectories::with_prefix("velouria").map_err(other_err)?;
    let state_dir = xdg_dirs
        .get_state_home()
        .into_os_string()
        .into_string()
        .expect("stringable path");

    // Veilid API state channel
    let (node_sender, node_receiver): (
        Sender<veilid_core::VeilidUpdate>,
        Receiver<veilid_core::VeilidUpdate>,
    ) = unbounded();

    // Start up Veilid core
    let update_callback = Arc::new(move |change: veilid_core::VeilidUpdate| {
        let _ = node_sender.send(change);
    });
    let config_callback = Arc::new(move |key| config::config_callback(state_dir.clone(), key));
    let api: veilid_core::VeilidAPI =
        veilid_core::api_startup(update_callback, config_callback).await?;
    api.attach().await?;

    // Wait for network to be up
    async {
        loop {
            let res = node_receiver.recv_async().await;
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
    .await?;

    // magic gonna happen here

    conn.execute("SELECT crsql_finalize()", []).optional()?;
    Ok(())
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
