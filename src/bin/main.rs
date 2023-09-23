use clap::Parser;

use ddcp::{
    cli::{Cli, Commands, RemoteArgs, RemoteCommands},
    Error, Result, DDCP,
};

#[tokio::main]
async fn main() {
    run().await.expect("ok");
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let mut app = DDCP::new(
        cli.db_file()?.as_str(),
        cli.state_dir()?.as_str(),
        cli.ext_file()?.as_str(),
    )
    .await?;

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
        Commands::Cleanup => Ok(()),
        Commands::Fetch { name } => app.fetch(name.as_str()).await,
        _ => Err(Error::Other("unsupported command".to_string())),
    };

    app.cleanup().await?;
    result
}
