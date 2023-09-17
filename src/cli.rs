use std::path::PathBuf;

use clap::{command, Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "velouria")]
#[command(bin_name = "velouria")]
pub struct Cli {
    #[arg(long)]
    pub app_dir: Option<String>,

    #[command(subcommand)]
    pub commands: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Init,
    Push,
    Fetch {
        #[arg(required = false)]
        name: String,
    },
    //Merge{ ... },
    //Pull{ ... },
    Remote(RemoteArgs),
    //Serve{ ... },
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

impl Cli {
    pub fn needs_network(&self) -> bool {
        match self.commands {
            Commands::Remote(_) => false,
            _ => true,
        }
    }

    pub fn path(&self, name: &str) -> String {
        must_path_as_string(PathBuf::from(self.app_dir()).join(name))
    }

    pub fn app_dir(&self) -> String {
        match &self.app_dir {
            Some(app_dir) => app_dir.to_owned(),
            None => must_path_as_string(self.base_dirs().get_state_home()),
        }
    }

    fn base_dirs(&self) -> xdg::BaseDirectories {
        xdg::BaseDirectories::with_prefix("velouria").expect("XDG base directories")
    }
}

fn must_path_as_string(p: PathBuf) -> String {
    p.as_os_str()
        .to_os_string()
        .into_string()
        .expect("string-able path")
}
