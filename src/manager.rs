use crate::config::Config;
use crate::watcher;

pub fn run() -> eyre::Result<()> {
    let config = Config::get()?;

    log::info!("Using config {config:?}");

    if let Err(error) = watcher::watch(&config.paths()) {
        log::error!("Error: {error:?}");
    }

    Ok(())
}
