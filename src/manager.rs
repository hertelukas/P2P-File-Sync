use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task;

use crate::config::Config;
use crate::file_sync::{do_full_scan, listen_file_sync, sync_files, try_connect, wait_incoming};
use crate::server::app;
use crate::{database, watcher};

pub async fn run() -> eyre::Result<()> {
    let config = Arc::new(Mutex::new(Config::get().await?));
    let pool = database::setup().await?;
    let pool = Arc::new(pool);

    let (tx_watch_cmd, rx_watch_cmd) = mpsc::channel(1);
    let (tx_change, mut rx_change) = mpsc::channel(1);
    let (tx_sync_cmd, rx_sync_cmd) = mpsc::channel(1);

    log::info!("Using config {:?}", config.lock().unwrap());

    for path in config.lock().unwrap().paths() {
        let _ = do_full_scan(pool.clone(), path.path(), path.id()).await?;
    }

    log::info!("Done scanning!");

    let watcher_config_handle = config.clone();
    // Task watching for changes
    task::spawn(async move {
        if let Err(error) = watcher::watch(watcher_config_handle, rx_watch_cmd, tx_change).await {
            log::error!("Error: {error:?}");
        }
    });

    // Task reacting to changes
    task::spawn(async move {
        while let Some(path) = rx_change.recv().await {
            handle_change(path).await;
        }
    });

    // Start trying to sync outdated files
    let syncer_config_handle = config.clone();
    let syncer_pool_handle = pool.clone();
    tokio::spawn(sync_files(
        syncer_pool_handle,
        syncer_config_handle,
        rx_sync_cmd,
    ));

    // And accept sync requests
    let listen_sync_mutex_handle = config.clone();
    tokio::spawn(listen_file_sync(listen_sync_mutex_handle));

    // Periodically try to connect to the peers
    let connector_config_handle = config.clone();
    let connector_pool_handle = pool.clone();
    let connector_sync_cmd_handle = tx_sync_cmd.clone();
    tokio::spawn(try_connect(
        connector_pool_handle,
        connector_config_handle,
        connector_sync_cmd_handle,
    ));
    // And listen if someone wants to connect to us
    let listener_config_handle = config.clone();
    let listener_pool_handle = pool.clone();
    let listener_sync_cmd_handle = tx_sync_cmd.clone();
    tokio::spawn(wait_incoming(
        listener_pool_handle,
        listener_config_handle,
        listener_sync_cmd_handle,
    ));

    let listener = TcpListener::bind("0.0.0.0:3617").await.unwrap();
    axum::serve(listener, app()).await.unwrap();

    Ok(())
}

async fn handle_change(path: PathBuf) {
    log::info!("Handling {path:?}");
}
