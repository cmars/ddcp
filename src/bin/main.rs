use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::prelude::*;

use ddcp::{
    cli::{Cli, Commands, RemoteArgs, RemoteCommands},
    other_err, warn_err, Error, Result, DDCP,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            EnvFilter::builder()
                .with_default_directive("ddcp=info".parse().unwrap())
                .from_env_lossy(),
        )
        .init();

    if let Err(_) = run().await {
        std::process::exit(1);
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

    let app_result = run_app(cli, &mut app).await;
    let _ = warn_err(app.cleanup().await, "cleanup failed");
    app_result
}

async fn run_app(cli: Cli, app: &mut DDCP) -> Result<()> {
    if cli.needs_network() {
        app.wait_for_network().await?;
    }

    let result = match cli.commands {
        Commands::Init => {
            let (addr, _site_id, version) = app.push().await?;
            info!(addr, db_version = version, "Registered database at DHT");
            Ok(())
        }
        Commands::Push => {
            let (addr, _site_id, version) = app.push().await?;
            info!(addr, db_version = version, "Pushed database status to DHT");
            Ok(())
        }
        Commands::Remote(RemoteArgs {
            commands: RemoteCommands::Add { name, addr },
        }) => {
            app.remote_add(name.clone(), addr.clone()).await?;
            info!(name, addr, "remote added");
            Ok(())
        }
        Commands::Remote(RemoteArgs {
            commands: RemoteCommands::Remove { name },
        }) => match app.remote_remove(name.clone()).await? {
            true => {
                info!(name, "remote removed");
                Ok(())
            }
            false => {
                error!(name, "not found");
                Err(other_err("failed to remove remote"))
            }
        },
        Commands::Remote(RemoteArgs {
            commands: RemoteCommands::List,
        }) => {
            let remotes = app.remotes();
            for remote in remotes.iter() {
                println!("{}\t{}", remote.0, remote.1);
            }
            Ok(())
        }
        Commands::Serve => return app.serve().await,
        Commands::Cleanup => Ok(()),
        Commands::Pull { name } => match name {
            Some(name) => {
                let _ = app.pull(name.as_str()).await?;
                Ok(())
            }
            None => {
                let remotes = app.remotes();
                let mut errors = vec![];
                for (name, _) in remotes.iter() {
                    if let Err(e) = warn_err(
                        app.pull(name).await,
                        format!("failed to pull from {}", name).as_str(),
                    ) {
                        errors.push(format!("{:?}", e));
                    }
                }
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(other_err(errors.join("\n")))
                }
            }
        },
        Commands::Addr => {
            println!("{}", app.addr());
            Ok(())
        }
        _ => Err(Error::Other("unsupported command".to_string())),
    };
    result
}
