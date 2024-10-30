use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use sha2::{Digest, Sha256};
use tokio::{
    fs::read,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use walkdir::WalkDir;

use crate::{
    config::Config,
    database::{insert, is_newer, is_tracked, update_if_newer},
    types::File,
};

type MutexConf = Arc<Mutex<Config>>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    IoError(#[from] tokio::io::Error),
    #[error(transparent)]
    DatabaseError(#[from] crate::database::Error),
}

fn hash_data(data: Vec<u8>) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Tries to connect to all peers
pub async fn try_connect(config: MutexConf) {
    loop {
        // Create a vector of owned peer copies first, so we do
        // not have to hold the lock over the await of connect
        let mut copied_peers = vec![];
        for peer in config.lock().unwrap().peer_ips() {
            copied_peers.push(peer.clone());
        }

        for peer in copied_peers {
            log::debug!("Trying to connect to {:?}", peer);
            if let Ok(stream) = TcpStream::connect((peer, 3618)).await {
                log::info!("Connected to {:?}", peer);
                let config_handle = config.clone();
                tokio::spawn(async move {
                    handle_connection(stream, config_handle).await;
                });
            }
        }
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}

pub async fn wait_incoming(config: MutexConf) {
    let listener = TcpListener::bind("0.0.0.0:3618").await.unwrap();

    log::info!("Listening on {:?}", listener.local_addr());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let config = config.clone();
                tokio::spawn(async move {
                    handle_connection(stream, config).await;
                });
            }
            Err(e) => log::warn!("Failed to accept connection {}", e),
        }
    }
}

async fn handle_connection(stream: TcpStream, config: MutexConf) {
    let peer_addr = match stream.peer_addr() {
        Ok(addr) => addr,
        Err(e) => {
            log::error!("Could not read peer address: {e}, dropping connection");
            return;
        },
    };

    let folders = match config.lock().unwrap().shared_folders(peer_addr.ip()) {
        Some(ids) => ids,
        None => {
            log::info!("Not sharing any folder with this peer. Dropping connection");
            return;
        }
    };

}

/// Updates the database by recursively iterating over all files in the path.
/// This is done by following these steps:
/// 1. Check if the file is tracked: If not, insert and done.
/// 2. Check if the file has a newer modified date. If not, done.
/// 3. Calculate the file hash and update the entry
pub async fn do_full_scan(pool: &sqlx::SqlitePool, path: &PathBuf) -> Result<(), Error> {
    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.metadata().map(|m| m.is_file()).unwrap_or(false))
    {
        // Insert the file in our database, if untracked
        if !is_tracked(pool, &entry.path()).await? {
            let content = read(entry.path()).await.map_err(Error::from)?;
            log::info!("Tracking {entry:?}");
            let f = File::new(hash_data(content), &entry);
            insert(pool, f).await.map_err(Error::from)?;
        }
        // We are tracking the file already, check for newer version
        else {
            // Is this worth it? Only useful if this is often false, otherwise, calculating
            // the hash might not be that big of an overhead
            if is_newer(pool, File::get_last_modified_as_unix(&entry), &entry.path()).await? {
                let content = read(entry.path()).await.map_err(Error::from)?;
                log::debug!("File has new modified date {entry:?}");
                update_if_newer(
                    pool,
                    File::get_last_modified_as_unix(&entry),
                    hash_data(content),
                    &entry.path(),
                )
                .await?;
            } else {
                log::debug!("No updated needed for {entry:?}");
            }
        }
    }

    Ok(())
}
