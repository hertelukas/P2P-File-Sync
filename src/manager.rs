use tokio::sync::mpsc;
use tokio::task;

use crate::config::Config;
use crate::watcher;

pub async fn run() -> eyre::Result<()> {
    let config = Config::get()?;

    let (tx_watch_cmd, rx_watch_cmd) = mpsc::channel(100);

    log::info!("Using config {config:?}");

    let t = task::spawn(async move {
        if let Err(error) = watcher::watch(&config.paths(), rx_watch_cmd).await {
            log::error!("Error: {error:?}");
        }
    });

    tx_watch_cmd.send(watcher::WatchCommand::Add {
        folder: crate::types::WatchedFolder::new("/tmp"),
    }).await.unwrap();

    let _ = tokio::time::sleep(std::time::Duration::new(5, 0)).await;

    tx_watch_cmd.send(watcher::WatchCommand::Remove {
        folder: crate::types::WatchedFolder::new("/tmp"),
    }).await.unwrap();

    let _ = t.await;

    Ok(())
}
