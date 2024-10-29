use std::path::PathBuf;

use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task;

use crate::config::Config;
use crate::file_sync::{do_full_scan, try_connect, wait_incoming};
use crate::{database, watcher};

pub async fn run() -> eyre::Result<()> {
    let config = Config::get().await?;
    let pool = database::setup().await?;

    let (tx_watch_cmd, rx_watch_cmd) = mpsc::channel(1);
    let (tx_change, mut rx_change) = mpsc::channel(1);

    log::info!("Using config {config:?}");

    for path in config.paths() {
        let _ = do_full_scan(&pool, path.path()).await?;
    }

    log::info!("Done scanning!");

    let peers = config.peer_ips().clone();

    // Task watching for changes
    task::spawn(async move {
        if let Err(error) = watcher::watch(&config.paths(), rx_watch_cmd, tx_change).await {
            log::error!("Error: {error:?}");
        }
    });

    // Task reacting to changes
    task::spawn(async move {
        while let Some(path) = rx_change.recv().await {
            handle_change(path).await;
        }
    });


    // Periodically try to connect to the peers
    tokio::spawn(try_connect(peers));
    // And listen if someone wants to connect to us
    wait_incoming().await;

    Ok(())
}

async fn handle_change(path: PathBuf) {
    log::info!("Handling {path:?}");
}
