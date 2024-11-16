use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task;

use crate::config::Config;
use crate::scan::{scan_file, scan_folder};
use crate::server::app;
use crate::sync::{announce_change, listen_file_sync, sync_files, try_connect, wait_incoming};
use crate::{database, watcher};

pub async fn run() -> eyre::Result<()> {
    let config = Arc::new(Mutex::new(Config::get().await?));
    let pool = database::setup().await?;
    let pool = Arc::new(pool);

    // Set all ports
    let http_server_port: u16 = std::env::var("HTTP_SERVER_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3617);
    let db_sync_port: u16 = std::env::var("DB_SERVER_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3618);
    let file_sync_port = std::env::var("DB_SERVER_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3619);
    let (tx_watch_cmd, rx_watch_cmd) = mpsc::channel(1);
    let (tx_change, mut rx_change) = mpsc::channel(1);
    let (tx_sync_cmd, rx_sync_cmd) = mpsc::channel(1);

    log::info!("Using config {:?}", config.lock().unwrap());

    for path in &*config.lock().unwrap().paths {
        let _ = scan_folder(pool.clone(), path.path(), path.id()).await?;
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
    let change_handler_pool_handle = pool.clone();
    let change_handler_config_handle = config.clone();
    task::spawn(async move {
        while let Some(path) = rx_change.recv().await {
            handle_change(
                change_handler_config_handle.clone(),
                change_handler_pool_handle.clone(),
                path,
                file_sync_port,
            )
            .await;
        }
    });

    // Start trying to sync outdated files
    let syncer_config_handle = config.clone();
    let syncer_pool_handle = pool.clone();
    tokio::spawn(sync_files(
        syncer_pool_handle,
        syncer_config_handle,
        rx_sync_cmd,
        file_sync_port,
    ));

    // And accept sync requests
    let listen_sync_config_handle = config.clone();
    let listen_sync_pool_handle = pool.clone();
    let tx_sync_cmd_handle = tx_sync_cmd.clone();
    tokio::spawn(listen_file_sync(
        listen_sync_pool_handle,
        listen_sync_config_handle,
        file_sync_port,
        tx_sync_cmd_handle,
    ));

    // Periodically try to connect to the peers
    let connector_config_handle = config.clone();
    let connector_pool_handle = pool.clone();
    let connector_sync_cmd_handle = tx_sync_cmd.clone();
    tokio::spawn(try_connect(
        connector_pool_handle,
        connector_config_handle,
        connector_sync_cmd_handle,
        db_sync_port,
    ));
    // And listen if someone wants to connect to us
    let listener_config_handle = config.clone();
    let listener_pool_handle = pool.clone();
    let listener_sync_cmd_handle = tx_sync_cmd.clone();
    tokio::spawn(wait_incoming(
        listener_pool_handle,
        listener_config_handle,
        listener_sync_cmd_handle,
        db_sync_port,
    ));

    let server_config_handle = config.clone();
    let listener = TcpListener::bind(format!("0.0.0.0:{}", http_server_port))
        .await
        .unwrap();
    log::info!("Listening for TUI on {:?}", listener.local_addr());
    axum::serve(listener, app(server_config_handle, tx_watch_cmd))
        .await
        .unwrap();

    Ok(())
}

async fn handle_change(
    config: Arc<Mutex<Config>>,
    pool: Arc<sqlx::SqlitePool>,
    path: PathBuf,
    port: u16,
) {
    let folder_id = {
        let lock = config.lock().unwrap();
        let files: Vec<_> = lock
            .paths
            .iter()
            .filter(|p| path.starts_with(&p.path))
            .collect();

        if files.len() != 1 {
            log::warn!("Cannot find folder_id for {path:?}");
            return;
        }
        files[0].id()
    };

    // Update database
    match scan_file(pool, &path, folder_id).await {
        Ok(file) => {
            // Propagate our changes to other peers, so they update their db
            // and then request the new file from us

            let (peers, base_path) = {
                let lock = config.lock().unwrap();
                (lock.peers.clone(), lock.get_path(folder_id))
            };

            if let Some(base_path) = base_path {
                if let Some(relative_path) = crate::types::File::get_relative_path(
                    &base_path.to_string_lossy(),
                    &path.to_string_lossy(),
                ) {
                    for peer in peers
                        .into_iter()
                        .filter(|p| p.folders.contains(&folder_id))
                        .collect::<Vec<_>>()
                    {
                        announce_change(
                            peer.ip,
                            file.clone(),
                            relative_path.to_string_lossy().to_string(),
                            folder_id,
                            port,
                        )
                        .await;
                    }
                } else {
                    log::warn!("Unable to load relative path for file: Not announcing changes");
                }
            } else {
                log::warn!("Cannot find base path of file");
            }
        }
        Err(e) => {
            log::warn!("Failed to react to update: {e}");
        }
    }
}
