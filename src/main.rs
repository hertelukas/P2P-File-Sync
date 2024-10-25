use env_logger::Env;
use p2p_file_sync::manager;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Use info as default logging level
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    color_eyre::install()?;
    manager::run().await
}
