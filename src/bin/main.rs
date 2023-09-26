use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::prelude::*;

use ddcp::{
    cli::{Cli, Commands, RemoteArgs, RemoteCommands},
    other_err, Error, Result, DDCP,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive("ddcp=info".parse().unwrap())
                .from_env_lossy(),
        )
        .init();

    match run().await {
        Ok(()) => {}
        Err(e) => error!("{:?}", e),
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let (db_file, state_dir, ext_file) = (cli.db_file()?, cli.state_dir()?, cli.ext_file()?);

    // Shell out to sqlite3 before other setup, if that's what we're doing
    if let Commands::Shell = cli.commands {
        let err = exec::Command::new("sqlite3")
            .arg("-cmd")
            .arg(format!(".load {}", ext_file).as_str())
            .arg("-bail")
            .arg(db_file)
            .exec();
        return Err(other_err(format!("failed to exec sqlite3: {:?}", err)));
    }

    let mut app = DDCP::new(
        Some(db_file.as_str()),
        state_dir.as_str(),
        ext_file.as_str(),
    )
    .await?;

    if cli.needs_network() {
        app.wait_for_network().await?;
    }

    let result = match cli.commands {
        Commands::Init => {
            let addr = app.init().await?;
            info!("{}", addr);
            Ok(())
        }
        Commands::Push => {
            let (addr, _site_id, version) = app.push().await?;
            info!("{} at version {}", addr, version);
            Ok(())
        }
        Commands::Remote(RemoteArgs {
            commands: RemoteCommands::Add { name, addr },
        }) => app.remote_add(name, addr).await,
        Commands::Remote(RemoteArgs {
            commands: RemoteCommands::Remove { name },
        }) => app.remote_remove(name).await,
        Commands::Remote(RemoteArgs {
            commands: RemoteCommands::List,
        }) => {
            let remotes = app.remotes().await?;
            for remote in remotes.iter() {
                info!("{}\t{}", remote.0, remote.1);
            }
            Ok(())
        }
        Commands::Serve => app.serve().await,
        Commands::Cleanup => Ok(()),
        Commands::Pull { name } => match name {
            Some(name) => {
                let _ = app.pull(name.as_str()).await?;
                Ok(())
            }
            None => {
                let remotes = app.remotes().await?;
                let mut errors = vec![];
                for (name, _) in remotes.iter() {
                    if let Err(e) = app.pull(name).await {
                        errors.push(format!("failed to fetch {}: {:?}", name, e));
                    }
                }
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(other_err(errors.join("\n")))
                }
            }
        },
        _ => Err(Error::Other("unsupported command".to_string())),
    };

    app.cleanup().await?;
    result
}
