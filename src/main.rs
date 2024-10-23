use p2p_file_sync::config::Config;
use p2p_file_sync::watcher;

fn main() ->eyre::Result<()> {
    env_logger::init();
    color_eyre::install()?;

    let config = Config::load()?;

    log::info!("Using config {config:?}");

    if let Err(error) = watcher::watch("/home/lukas/Downloads") {
        log::error!("Error: {error:?}");
    }

    Ok(())
}
