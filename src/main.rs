use p2p_file_sync::config::Config;
use p2p_file_sync::watcher;

fn main() -> eyre::Result<()> {
    env_logger::init();
    color_eyre::install()?;

    let config = match Config::load() {
        Ok(conf) => conf,
        // TODO probably not so good idea, because we just create a new one
        // if anything fails
        Err(e) => {
            log::warn!("{}", e);
            Config::create_default()?
        }
    };

    log::info!("Using config {config:?}");

    if let Err(error) = watcher::watch("/home/lukas/Downloads") {
        log::error!("Error: {error:?}");
    }

    Ok(())
}
