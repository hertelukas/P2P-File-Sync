use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::StreamExt;
use sha2::{Digest, Sha256};
use sqlx::query_as;
use tokio::{
    fs::read,
    net::{TcpListener, TcpStream},
    sync::mpsc::{Receiver, Sender},
};
use walkdir::WalkDir;

use crate::{
    config::Config,
    connection::Connection,
    database::{insert, is_newer, is_tracked, update_if_newer},
    frame::Frame,
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

/// Helper function which returns the Sha256 of `data`
fn hash_data(data: Vec<u8>) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Tries to connect to all peers. We try to connect to peers
/// within a fixed interval, in the case that a peer comes online
/// without trying to connect to us first.
///
/// If a connetion is established, a new thread is spawned,
/// responsible for handling the connection.
pub async fn try_connect(pool: Arc<sqlx::SqlitePool>, config: MutexConf, tx_sync_cmd: Sender<()>) {
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
                let pool_handle = pool.clone();
                let tx_handle = tx_sync_cmd.clone();
                tokio::spawn(async move {
                    handle_connection(stream, config_handle, pool_handle, true, tx_handle).await;
                });
            }
        }
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}

/// Listens to new incoming connections. On connection establishment,
/// it is handled in a new thread.
pub async fn wait_incoming(
    pool: Arc<sqlx::SqlitePool>,
    config: MutexConf,
    tx_sync_cmd: Sender<()>,
) {
    let listener = TcpListener::bind("0.0.0.0:3618").await.unwrap();

    log::info!("Listening on {:?}", listener.local_addr());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let config = config.clone();
                let pool = pool.clone();
                let tx_handle = tx_sync_cmd.clone();

                tokio::spawn(async move {
                    handle_connection(stream, config, pool, false, tx_handle).await;
                });
            }
            Err(e) => log::warn!("Failed to accept connection {}", e),
        }
    }
}

/// Handles newly established connections. If we do not share any
/// folder with the IP, the connection is dropped.
///
/// Otherwise, if `initiator`, we start synchronizing the database.
/// If not, start listening for synchronization requests.
async fn handle_connection(
    stream: TcpStream,
    config: MutexConf,
    pool: Arc<sqlx::SqlitePool>,
    initiator: bool,
    tx_sync_cmd: Sender<()>,
) {
    let peer_addr = match stream.peer_addr() {
        Ok(addr) => addr,
        Err(e) => {
            log::error!("Could not read peer address: {e}, dropping connection");
            return;
        }
    };

    let folders = match config.lock().unwrap().shared_folders(peer_addr.ip()) {
        Some(ids) => ids,
        None => {
            log::info!(
                "Not sharing any folder with {:?}. Dropping connection",
                peer_addr.ip()
            );
            return;
        }
    };
    let mut connection = Connection::new(stream);

    if initiator {
        // Send folder info first
        for folder in folders {
            send_db_state(&mut connection, folder, config.clone(), pool.clone()).await;
            connection.write_frame(&Frame::Done).await.unwrap();
        }
        connection.write_frame(&Frame::Done).await.unwrap();
        // and then receive updates
        receive_db_state(&mut connection, config.clone(), pool.clone()).await;
    } else {
        receive_db_state(&mut connection, config.clone(), pool.clone()).await;
        for folder in folders {
            send_db_state(&mut connection, folder, config.clone(), pool.clone()).await;
            connection.write_frame(&Frame::Done).await.unwrap();
        }
        connection.write_frame(&Frame::Done).await.unwrap();
    }
    // Notify our syncer
    tx_sync_cmd.send(()).await.unwrap();
}

/// This function should be called for each folder which should
/// be synced with `connection`. It updates our and the peers database
/// in order to share the same global database state for folder `folder_id`
async fn send_db_state(
    connection: &mut Connection,
    folder_id: u32,
    config: MutexConf,
    pool: Arc<sqlx::SqlitePool>,
) {
    // Announce our folder
    connection
        .write_frame(&Frame::DbSync { folder_id })
        .await
        .unwrap();

    // Wait for yes or no
    if let Some(response) = connection.read_frame().await.unwrap() {
        match response {
            Frame::Yes => {
                // Load the path (and hold the lock as short as possible,
                // because we can't Send it to another await)
                let path = {
                    let lock = config.lock().unwrap();
                    lock.get_path(folder_id)
                };
                if let Some(path) = path {
                    // All files that start with the same folder
                    let like_pattern = format!("{}%", path.to_string_lossy());
                    let mut file_stream = query_as!(
                        File,
                        r#"
SELECT *
FROM files
WHERE path LIKE ?
"#,
                        like_pattern
                    )
                    .fetch(&*pool);

                    // Now, we can send our file state
                    while let Some(Ok(file)) = file_stream.next().await {
                        if let Some(relative_path) =
                            File::get_relative_path(&path.to_string_lossy(), &file.path)
                        {
                            let file_frame = Frame::InitiatorGlobal {
                                global_hash: file.global_hash.into(),
                                global_last_modified: file.global_last_modified,
                                global_peer: file.global_peer,
                                path: relative_path.to_string_lossy().to_string(),
                            };
                            connection.write_frame(&file_frame).await.unwrap();
                        } else {
                            log::warn!("Failed to convert path to a relative path, skipping");
                        }

                        // Check if we can continue
                        if let Some(response) = connection.read_frame().await.unwrap() {
                            match response {
                                Frame::Yes => (),
                                _ => log::warn!(
                                    "Unexpected packet received after sending file state"
                                ),
                            }
                        }
                    }
                } else {
                    // This should never happen
                    log::warn!("Tried to sync non-existing folder {folder_id}");
                    return;
                }
            }
            Frame::No => {
                log::info!("Peer does not want to sync {folder_id}");
                return;
            }
            _ => {
                log::warn!(
                    "Unexpected frame {:?} received while waiting if peer is interested in folder",
                    response
                );
                return;
            }
        }
    }
}

/// Waits for folder sync requests. If we are interested in the folders'
/// global state, we iterate over the file information.
async fn receive_db_state(
    connection: &mut Connection,
    config: MutexConf,
    pool: Arc<sqlx::SqlitePool>,
) {
    // Iterate over the folders that the initiator might want to share
    while let Some(frame) = connection.read_frame().await.unwrap() {
        match frame {
            Frame::DbSync { folder_id } => {
                if config
                    .lock()
                    .unwrap()
                    .is_shared_with(connection.get_peer_ip().unwrap(), folder_id)
                {
                    // Accept folder information
                    let _ = connection.write_frame(&Frame::Yes).await;
                    let base_path = {
                        let lock = config.lock().unwrap();
                        lock.get_path(folder_id)
                    };
                    if let Some(base_path) = base_path {
                        while let Some(frame) = connection.read_frame().await.unwrap() {
                            match frame {
                                Frame::InitiatorGlobal {
                                    global_hash,
                                    global_last_modified,
                                    global_peer,
                                    path,
                                } => {
                                    let full_path =
                                        File::get_full_path(&base_path.to_string_lossy(), &path)
                                            .to_string_lossy()
                                            .to_string();
                                    let peer_addr = if global_peer == "0" {
                                        connection.get_peer_ip().unwrap().to_string()
                                    } else {
                                        global_peer
                                    };

                                    let hash_as_vec: Vec<u8> = global_hash.clone().into();
                                    let res = sqlx::query!(
                                        r#"
SELECT global_last_modified, global_hash
FROM files
WHERE path = ?
"#,
                                        full_path
                                    )
                                    .fetch_one(&*pool)
                                    .await;

                                    if let Ok(res) = res {
                                        // Need to update our database
                                        if res.global_hash != global_hash
                                            && res.global_last_modified < global_last_modified
                                        {
                                            log::info!("We need to update {full_path}");
                                            sqlx::query!(
                                                r#"
UPDATE files
SET global_hash = ?, global_last_modified = ?, global_peer = ?
WHERE path = ?
"#,
                                                hash_as_vec,
                                                global_last_modified,
                                                peer_addr,
                                                full_path
                                            )
                                            .execute(&*pool)
                                            .await
                                            .unwrap();
                                        }
                                    }
                                    // Need to insert file
                                    else {
                                        log::info!("We are not yet tracking {full_path}");
                                        sqlx::query!(
                                            r#"
INSERT INTO files (path, global_hash, global_last_modified, global_peer)
VALUES (?, ?, ?, ?)
"#,
                                            full_path,
                                            hash_as_vec,
                                            global_last_modified,
                                            peer_addr
                                        )
                                        .execute(&*pool)
                                        .await
                                        .unwrap();
                                    }
                                    // And wait for the next file
                                    connection.write_frame(&Frame::Yes).await.unwrap();
                                }
                                Frame::Done => {
                                    log::info!("Received information for all files");
                                    break;
                                }
                                _ => log::warn!(
                                "Unexpected packet received while waiting for new file information"
                            ),
                            }
                        }
                    } else {
                        // Should never happen
                        log::warn!("Tried to sync non-exisitng folder {folder_id}");
                    }
                } else {
                    // If we do not want this folder information, we expect a new DbSync frame
                    let _ = connection.write_frame(&Frame::No).await;
                }
            }
            Frame::Done => {
                log::info!("Received all folder information from peer");
                return;
            }
            _ => {
                log::warn!(
                    "Unexpected frame {:?} received while waiting for new folder",
                    frame
                );
            }
        }
    }
}

pub async fn sync_files(pool: Arc<sqlx::SqlitePool>, mut rx: Receiver<()>) {
    loop {
        // Block on waiting for some change that motivates us to sync
        rx.recv().await;
        log::info!("Starting file sync...");

        let mut file_stream = sqlx::query_as!(
            File,
            r#"
SELECT *
FROM files
WHERE (global_hash <> local_hash) OR (local_hash IS NULL)
"#
        )
        .fetch(&*pool);

        while let Some(Ok(file)) = file_stream.next().await {
            log::info!("Trying to sync {:?}", file);
            // We should never have a newer local file where
            // we do not own the global state
            if let Some(local_last_modified) = file.local_last_modified {
                if local_last_modified > file.global_last_modified {
                    log::error!("Encountered file with inconsistent state: {:?}", file);
                }
            }
        }
    }
}

/// Updates the database by recursively iterating over all files in the path.
/// This is done by following these steps:
/// 1. Check if the file is tracked: If not, insert and done.
/// 2. Check if the file has a newer modified date. If not, done.
/// 3. Calculate the file hash and update
pub async fn do_full_scan(pool: Arc<sqlx::SqlitePool>, path: &PathBuf) -> Result<(), Error> {
    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.metadata().map(|m| m.is_file()).unwrap_or(false))
    {
        // Insert the file in our database, if untracked
        // TODO maybe this has to be changed, as is_tracked will return true,
        // even if we only track the global state of the file: We should be inserting
        // it anyway
        if !is_tracked(&pool, &entry.path()).await? {
            let content = read(entry.path()).await.map_err(Error::from)?;
            log::info!("Tracking {entry:?}");
            let f = File::new(hash_data(content), &entry);
            insert(&pool, f).await.map_err(Error::from)?;
        }
        // We are tracking the file already, check for newer version
        else {
            // Is this worth it? Only useful if this is often false, otherwise, calculating
            // the hash might not be that big of an overhead
            if is_newer(
                &pool,
                File::get_last_modified_as_unix(&entry),
                &entry.path(),
            )
            .await?
            {
                let content = read(entry.path()).await.map_err(Error::from)?;
                log::debug!("File has new modified date {entry:?}");
                update_if_newer(
                    &pool,
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
