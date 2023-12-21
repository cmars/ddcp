use std::{
    fs,
    path::{Path, PathBuf},
};

use clap::{command, Args, Parser, Subcommand};

use crate::error::{other_err, Error, Result};

#[derive(Parser, Debug)]
#[command(name = "ddcp")]
#[command(bin_name = "ddcp")]
pub struct Cli {
    // CR-SQLite database file to replicate and synchronize. Defaults to
    // "ddcp.db" in the current working directory, if not specified.
    #[arg(long, env)]
    pub db_file: Option<String>,

    // State directory, which stores remote peers, Veilid secret keys and
    // routing information. Defaults to ".ddcp/$(basename $DDCP_FILE)" in the
    // same directory containing $DDCP_FILE.
    #[arg(long, env)]
    pub state_dir: Option<String>,

    /// File location of the cr-sqlite shared library extension file. If not
    /// specified, ddcp will attempt to load "crsqlite.so" from the same
    /// directory containing the running "ddcp" executable.
    #[arg(long, env)]
    pub ext_file: Option<String>,

    #[command(subcommand)]
    pub commands: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Init,
    Push,
    Fetch {
        #[arg(required = false)]
        name: Option<String>,
    },
    Merge {
        #[arg(required = true)]
        name: String,
    },
    Pull {
        #[arg(required = false)]
        name: Option<String>,
    },
    Remote(RemoteArgs),
    Serve,
    Shell,
    Addr,
    Cleanup,
}

#[derive(Args, Debug)]
pub struct RemoteArgs {
    #[command(subcommand)]
    pub commands: RemoteCommands,
}

#[derive(Subcommand, Debug)]
pub enum RemoteCommands {
    Add {
        #[arg(required = true)]
        name: String,
        #[arg(required = true)]
        addr: String,
    },
    Remove {
        #[arg(required = true)]
        name: String,
    },
    List,
    Status {
        #[arg(required = true)]
        name: String,
    },
}

const DEFAULT_DB_FILENAME: &'static str = "ddcp.db";
const DEFAULT_STATE_DIRNAME: &'static str = ".ddcp";

impl Cli {
    pub fn needs_network(&self) -> bool {
        match self.commands {
            Commands::Remote(_) => false,
            Commands::Shell => false,
            _ => true,
        }
    }

    pub fn db_file(&self) -> Result<String> {
        let result = match self.db_file {
            Some(ref f) => Ok(f.to_string()),
            None => path_as_string(Path::join(
                &std::env::current_dir().map_err(other_err)?,
                DEFAULT_DB_FILENAME,
            )),
        }?;
        let parent = match Path::new(&result).parent() {
            Some(p) => p,
            None => {
                return Err(other_err(format!(
                    "failed to locate parent directory of {:?}",
                    result
                )));
            }
        };
        if !parent.try_exists()? {
            fs::DirBuilder::new().recursive(true).create(parent)?;
        }
        Ok(result)
    }

    pub fn state_dir(&self) -> Result<String> {
        let db_file = self.db_file()?;
        let db_path = Path::new(&db_file);
        let state_dir = match self.state_dir {
            Some(ref d) => Ok::<String, Error>(d.to_string()),
            None => path_as_string(Path::join(
                &Path::join(
                    match db_path.parent() {
                        Some(p) => p,
                        None => {
                            return Err(other_err(format!(
                                "failed to infer state path from {:?}",
                                db_path
                            )))
                        }
                    },
                    DEFAULT_STATE_DIRNAME,
                ),
                match db_path.file_name() {
                    Some(p) => p,
                    None => {
                        return Err(other_err(format!(
                            "failed to infer state path from {:?}",
                            db_path
                        )))
                    }
                },
            )),
        }?;
        let state_path = Path::new(&state_dir);
        if !state_path.try_exists()? {
            fs::DirBuilder::new().recursive(true).create(state_path)?;
        }
        Ok(state_dir)
    }

    pub fn ext_file(&self) -> Result<String> {
        let ext_file = match self.ext_file {
            Some(ref f) => Ok::<String, Error>(f.to_string()),
            None => {
                let exe = std::env::current_exe()?;
                let exe_dir = exe.parent().expect("executable has a parent directory");
                Ok(String::from(
                    exe_dir
                        // TODO: dylib on macos, dll on windows?
                        .join(PathBuf::from("crsqlite.so"))
                        .as_os_str()
                        .to_str()
                        .expect("valid path string"),
                ))
            }
        }?;
        let ext_path = Path::new(&ext_file);
        if !ext_path.exists() {
            return Err(other_err(format!(
                "database extension {} not found",
                ext_file
            )));
        }
        Ok(match ext_file.strip_suffix(".so") {
            Some(s) => s.to_string(),
            None => ext_file,
        })
    }
}

pub fn path_as_string(p: PathBuf) -> Result<String> {
    p.as_os_str()
        .to_os_string()
        .into_string()
        .map_err(|oss| other_err(format!("failed to render path {:?}", oss)))
}
