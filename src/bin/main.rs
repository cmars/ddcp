use clap::Parser;

use ddcp::{
    cli::{Cli, Commands, RemoteArgs, RemoteCommands},
    Error, Result, DDCP, other_err,
};

#[tokio::main]
async fn main() {
    run().await.expect("ok");
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let (db_file, state_dir, ext_file) = (cli.db_file()?, cli.state_dir()?, cli.ext_file()?);
    let mut app = DDCP::new(db_file.as_str(), state_dir.as_str(), ext_file.as_str()).await?;

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
        }) => {
            let remotes = app.remotes().await?;
            for remote in remotes.iter() {
                println!("{}\t{}", remote.0, remote.1);
            }
            Ok(())
        }
        Commands::Serve => app.serve().await,
        Commands::Shell => {
            let err = exec::Command::new("sqlite3")
                .arg("-cmd")
                .arg(format!(".load {}", ext_file).as_str())
                .arg("-bail")
                .arg(db_file)
                .exec();
            Err(other_err(format!("failed to exec sqlite3: {:?}", err)))
        }
        Commands::Cleanup => Ok(()),
        Commands::Fetch { name } => app.fetch(name.as_str()).await,
        _ => Err(Error::Other("unsupported command".to_string())),
    };

    app.cleanup().await?;
    result
}
