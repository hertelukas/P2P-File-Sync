use std::path::PathBuf;

use tokio::sync::mpsc;
use tokio::task;

use crate::config::Config;
use crate::watcher;

pub async fn run() -> eyre::Result<()> {
    let config = Config::get().await?;

    let (tx_watch_cmd, rx_watch_cmd) = mpsc::channel(1);
    let (tx_change, mut rx_change) = mpsc::channel(1);

    log::info!("Using config {config:?}");

    let watcher_task = task::spawn(async move {
        if let Err(error) = watcher::watch(&config.paths(), rx_watch_cmd, tx_change).await {
            log::error!("Error: {error:?}");
        }
    });

    task::spawn(async move {
        while let Some(path) = rx_change.recv().await {
            handle_change(path).await;
        }
    });

    tx_watch_cmd
        .send(watcher::WatchCommand::Add {
            folder: crate::types::WatchedFolder::new("/tmp"),
        })
        .await
        .unwrap();

    let _ = tokio::time::sleep(std::time::Duration::new(5, 0)).await;

    tx_watch_cmd
        .send(watcher::WatchCommand::Remove {
            folder: crate::types::WatchedFolder::new("/tmp"),
        })
        .await
        .unwrap();

    let _ = watcher_task.await;

    Ok(())
}

async fn handle_change(path: PathBuf) {
    log::info!("Handling {path:?}");
}
