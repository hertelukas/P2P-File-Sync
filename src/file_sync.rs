use std::{
    fs,
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
pub async fn try_connect(
    pool: Arc<sqlx::SqlitePool>,
    config: MutexConf,
    tx_sync_cmd: Sender<()>,
    port: u16,
) {
    loop {
        // Create a vector of owned peer copies first, so we do
        // not have to hold the lock over the await of connect
        let mut copied_peers = vec![];
        for peer in config.lock().unwrap().peer_ips() {
            copied_peers.push(peer.clone());
        }

        for peer in copied_peers {
            log::debug!("Trying to connect to {:?}", peer);
            if let Ok(stream) = TcpStream::connect((peer, port)).await {
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
    port: u16,
) {
    wait_incoming_intern(pool, config, tx_sync_cmd, port, false).await;
}

/// Internal function which allows us to test a single connection
async fn wait_incoming_intern(
    pool: Arc<sqlx::SqlitePool>,
    config: MutexConf,
    tx_sync_cmd: Sender<()>,
    port: u16,
    once: bool,
) {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .unwrap();

    log::info!("Listening for database sync on {:?}", listener.local_addr());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let config = config.clone();
                let pool = pool.clone();
                let tx_handle = tx_sync_cmd.clone();

                let t = tokio::spawn(async move {
                    handle_connection(stream, config, pool, false, tx_handle).await;
                });

                if once {
                    t.await.unwrap();
                    break;
                }
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
WHERE (folder_id = ?) AND (path LIKE ?)
"#,
                        folder_id,
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
WHERE folder_id = ? AND path = ?
"#,
                                        folder_id,
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
WHERE folder_id = ? AND path = ?
"#,
                                                hash_as_vec,
                                                global_last_modified,
                                                peer_addr,
                                                folder_id,
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
INSERT INTO files (folder_id, path, global_hash, global_last_modified, global_peer)
VALUES (?, ?, ?, ?, ?)
"#,
                                            folder_id,
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

/// Synchronizes the file state when `rx` receives a message.
pub async fn sync_files(
    pool: Arc<sqlx::SqlitePool>,
    config: MutexConf,
    rx: Receiver<()>,
    port: u16,
) {
    sync_files_internal(pool, config, rx, port, false).await;
}

/// Synchorinzes all files which are not at the newest state when
/// receiving a message from `rx`. If `once` is true, we will
/// only synchronize the files which are out-of-date in our database,
/// but will not wait for later resyncs.
async fn sync_files_internal(
    pool: Arc<sqlx::SqlitePool>,
    config: MutexConf,
    mut rx: Receiver<()>,
    port: u16,
    once: bool,
) {
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
            if let Some(local_last_modified) = &file.local_last_modified {
                if local_last_modified > &file.global_last_modified {
                    log::error!("Encountered file with inconsistent state: {:?}", file);
                }
            }

            // TODO We currently do a connection for each file, maybe improve
            // Maybe a bit inefficient if the peer is offline
            if let Ok(stream) = TcpStream::connect((file.global_peer.clone(), port)).await {
                log::info!("Connected to {:?} for file sync", file.global_peer);
                let base_path: PathBuf = {
                    let lock = config.lock().unwrap();
                    lock.get_path(file.folder_id.try_into().unwrap()).unwrap()
                };

                let mut connection = Connection::new(stream);
                connection
                    .write_frame(&Frame::RequestFile {
                        folder_id: file.folder_id.try_into().unwrap(),
                        path: File::get_relative_path(
                            &base_path.to_string_lossy(),
                            &file.path.clone(),
                        )
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    })
                    .await
                    .unwrap();

                if let Some(frame) = connection.read_frame().await.unwrap() {
                    match frame {
                        Frame::File { size: _, data } => {
                            let path =
                                File::get_full_path(&base_path.to_string_lossy(), &file.path);
                            fs::write(path, data).unwrap();
                        }
                        _ => log::warn!(
                            "Unexpected frame {:?} received while waiting for file request",
                            frame
                        ),
                    }
                }
            }
        }
        if once {
            return;
        }
    }
}

pub async fn listen_file_sync(config: MutexConf, port: u16) {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .unwrap();
    log::info!("Listening for file sync on {:?}", listener.local_addr());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let config = config.clone();
                tokio::spawn(async move {
                    let mut connection = Connection::new(stream);
                    if let Some(frame) = connection.read_frame().await.unwrap() {
                        match frame {
                            Frame::RequestFile { folder_id, path } => {
                                let base_path = {
                                    let lock = config.lock().unwrap();
                                    lock.get_path(folder_id)
                                };
                                if let Some(base_path) = base_path {
                                    let path =
                                        File::get_full_path(&base_path.to_string_lossy(), &path);

                                    // TODO handle read failure
                                    let data = fs::read(path).unwrap();
                                    connection
                                        .write_frame(&Frame::File {
                                            size: data.len().try_into().unwrap(),
                                            data: data.into(),
                                        })
                                        .await
                                        .unwrap();
                                }
                            }
                            _ => log::warn!(
                                "Unexpected frame {:?} received while waiting for file request",
                                frame
                            ),
                        }
                    }
                });
            }
            Err(e) => log::warn!("Failed to accept connection {}", e),
        }
    }
}

/// Updates the database by recursively iterating over all files in the path.
/// This is done by following these steps:
/// 1. Check if the file is tracked: If not, insert and done.
/// 2. Check if the file has a newer modified date. If not, done.
/// 3. Calculate the file hash and update
pub async fn do_full_scan(
    pool: Arc<sqlx::SqlitePool>,
    path: &PathBuf,
    folder_id: u32,
) -> Result<(), Error> {
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
            let f = File::new(folder_id, hash_data(content), &entry);
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

#[cfg(test)]
mod tests {
    use sqlx::{
        sqlite::{SqliteConnectOptions, SqlitePoolOptions},
        SqlitePool,
    };
    use std::io::{Read, Write};
    use std::{fs::File, str::FromStr};
    use tempfile::{tempdir, TempDir};
    use tokio::sync::mpsc;

    use crate::{Peer, WatchedFolder};

    use super::*;

    /// Enables logging during test execution
    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }
    /// Creates a new temporary directory.
    ///
    /// When TempDir goes out of scope, it may fail to delete the
    /// directory, if we still have handles to files.
    fn create_test_dir() -> TempDir {
        let tmp_dir = tempdir().unwrap();

        let foo_file_path = tmp_dir.path().join("foo.txt");
        let bar_file_path = tmp_dir.path().join("bar.txt");

        let mut foo_file = File::create(foo_file_path).unwrap();
        let mut bar_file = File::create(bar_file_path).unwrap();

        writeln!(foo_file, "foo").unwrap();
        writeln!(bar_file, "bar").unwrap();

        // Okay to not explicitly drop the files, as tmpdir will live on
        tmp_dir
    }

    async fn get_all_files(pool: &SqlitePool) -> Vec<crate::types::File> {
        query_as!(
            crate::types::File,
            r#"
SELECT *
FROM files
"#
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// Creates two empty databases, so we can test database synchronization
    async fn create_two_dbs() -> (Arc<SqlitePool>, Arc<SqlitePool>) {
        let client_db_connection_options =
            SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let server_db_connection_options =
            SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let client_pool = Arc::new(
            SqlitePoolOptions::new()
                .connect_with(client_db_connection_options)
                .await
                .unwrap(),
        );

        let server_pool = Arc::new(
            SqlitePoolOptions::new()
                .connect_with(server_db_connection_options)
                .await
                .unwrap(),
        );

        sqlx::migrate!().run(&*client_pool).await.unwrap();
        sqlx::migrate!().run(&*server_pool).await.unwrap();

        (client_pool, server_pool)
    }

    #[sqlx::test]
    async fn test_normal_scan(pool: SqlitePool) {
        let dir = create_test_dir();
        let pool = Arc::new(pool);
        do_full_scan(pool.clone(), &dir.path().to_path_buf(), 6)
            .await
            .unwrap();

        let files = get_all_files(&*pool).await;
        assert_eq!(files.len(), 2);

        files.into_iter().for_each(|f| {
            assert!(f.folder_id == 6);
            assert!(f
                .path
                .starts_with(dir.path().to_path_buf().to_string_lossy().as_ref()));
            assert!(f.local_hash.is_some());
            assert!(f.local_last_modified.is_some());
        });
    }

    #[sqlx::test]
    async fn test_empty_dir_scan(pool: SqlitePool) {
        let dir = tempdir().unwrap();
        let pool = Arc::new(pool);
        do_full_scan(pool.clone(), &dir.path().to_path_buf(), 6)
            .await
            .unwrap();

        let files = get_all_files(&*pool).await;
        assert_eq!(files.len(), 0);
    }

    #[sqlx::test]
    async fn test_empty_file_scan(pool: SqlitePool) {
        let dir = tempdir().unwrap();
        let foo_file_path = dir.path().join("foo.txt");
        let foo_file = File::create(foo_file_path).unwrap();
        // Probably unnecessary? Not sure if value is dropped if unnamed though
        drop(foo_file);

        let pool = Arc::new(pool);
        do_full_scan(pool.clone(), &dir.path().to_path_buf(), 6)
            .await
            .unwrap();

        let files = get_all_files(&*pool).await;
        assert_eq!(files.len(), 1);
        assert!(files.into_iter().all(|f| f.path.contains("foo.txt")));
    }

    #[sqlx::test]
    async fn test_recursive_folder(pool: SqlitePool) {
        let dir = create_test_dir();
        let pool = Arc::new(pool);
        let nested_dir_path = dir.path().join("sub");
        fs::create_dir(&nested_dir_path).unwrap();

        let third_path = nested_dir_path.join("third");
        let third_file = File::create(third_path).unwrap();
        // Needed, so folder can be deleted at the end
        drop(third_file);
        do_full_scan(pool.clone(), &dir.path().to_path_buf(), 6)
            .await
            .unwrap();

        let files = get_all_files(&*pool).await;

        assert_eq!(files.len(), 3);
        assert!(files.into_iter().any(|f| f.path.contains("third")))
    }

    #[tokio::test]
    async fn test_sync_db() {
        init();
        let (client_pool, server_pool) = create_two_dbs().await;

        let client_dir = create_test_dir();
        do_full_scan(client_pool.clone(), &client_dir.path().to_path_buf(), 1)
            .await
            .unwrap();

        assert_eq!(get_all_files(&*client_pool).await.len(), 2);
        assert_eq!(get_all_files(&*server_pool).await.len(), 0);

        let client_config = Arc::new(Mutex::new(Config {
            paths: vec![WatchedFolder::new_full(1, client_dir.path())],
            peers: vec![Peer {
                ip: [127, 0, 0, 1].into(),
                folders: vec![1],
            }],
        }));

        let server_dir = tempdir().unwrap();
        let server_config = Arc::new(Mutex::new(Config {
            paths: vec![WatchedFolder::new_full(1, server_dir.path())],
            peers: vec![Peer {
                ip: [127, 0, 0, 1].into(),
                folders: vec![1],
            }],
        }));

        let (tx_client_sync_cmd, mut rx_client) = mpsc::channel(1);
        let (tx_server_sync_cmd, mut rx_server) = mpsc::channel(1);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(_) = rx_client.recv() => {},
                    Some(_) = rx_server.recv() => {}
                }
            }
        });
        // Client
        let client_pool_handle = client_pool.clone();
        tokio::spawn(async move {
            try_connect(client_pool_handle, client_config, tx_client_sync_cmd, 1234).await;
        });

        wait_incoming_intern(
            server_pool.clone(),
            server_config,
            tx_server_sync_cmd,
            1234,
            true,
        )
        .await;

        // Make sure the "server" has all the latest info
        assert_eq!(get_all_files(&*server_pool).await.len(), 2);
        assert!(get_all_files(&*server_pool)
            .await
            .into_iter()
            .any(|f| f.path.contains("foo.txt")));
        assert!(get_all_files(&*server_pool)
            .await
            .into_iter()
            .any(|f| f.path.contains("bar.txt")));
        assert!(get_all_files(&*server_pool)
            .await
            .into_iter()
            .all(|f| f.local_hash.is_none()));
        assert!(get_all_files(&*server_pool)
            .await
            .into_iter()
            .all(|f| f.local_last_modified.is_none()));
        assert!(get_all_files(&*server_pool)
            .await
            .into_iter()
            .all(|f| f.global_peer == "127.0.0.1"));

        // Make sure the "client" did not change
        assert!(get_all_files(&*client_pool)
            .await
            .into_iter()
            .all(|f| f.global_peer == "0"));
        assert_eq!(get_all_files(&*client_pool).await.len(), 2)
    }

    #[tokio::test]
    async fn test_sync_empty_db() {
        init();
        let (client_pool, server_pool) = create_two_dbs().await;

        let client_dir = tempdir().unwrap();
        do_full_scan(client_pool.clone(), &client_dir.path().to_path_buf(), 1)
            .await
            .unwrap();

        assert_eq!(get_all_files(&*client_pool).await.len(), 0);
        assert_eq!(get_all_files(&*server_pool).await.len(), 0);

        let client_config = Arc::new(Mutex::new(Config {
            paths: vec![WatchedFolder::new_full(1, client_dir.path())],
            peers: vec![Peer {
                ip: [127, 0, 0, 1].into(),
                folders: vec![1],
            }],
        }));

        let server_dir = tempdir().unwrap();
        let server_config = Arc::new(Mutex::new(Config {
            paths: vec![WatchedFolder::new_full(1, server_dir.path())],
            peers: vec![Peer {
                ip: [127, 0, 0, 1].into(),
                folders: vec![1],
            }],
        }));

        let (tx_client_sync_cmd, mut rx_client) = mpsc::channel(1);
        let (tx_server_sync_cmd, mut rx_server) = mpsc::channel(1);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(_) = rx_client.recv() => {},
                    Some(_) = rx_server.recv() => {}
                }
            }
        });
        // Client
        let client_pool_handle = client_pool.clone();
        tokio::spawn(async move {
            try_connect(client_pool_handle, client_config, tx_client_sync_cmd, 1235).await;
        });

        wait_incoming_intern(
            server_pool.clone(),
            server_config,
            tx_server_sync_cmd,
            1235,
            true,
        )
        .await;

        // "Server" still should have nothing
        assert_eq!(get_all_files(&*server_pool).await.len(), 0);

        // "Client" neither
        assert_eq!(get_all_files(&*client_pool).await.len(), 0)
    }

    #[tokio::test]
    async fn test_sync_mutual_db() {
        init();
        let (client_pool, server_pool) = create_two_dbs().await;

        let client_dir = create_test_dir();
        do_full_scan(client_pool.clone(), &client_dir.path().to_path_buf(), 1)
            .await
            .unwrap();

        assert_eq!(get_all_files(&*client_pool).await.len(), 2);

        // Also fill the server with a couple of files
        let server_dir = tempdir().unwrap();

        let foo_file_path = server_dir.path().join("foo2.txt");
        let bar_file_path = server_dir.path().join("bar2.txt");
        let baz_file_path = server_dir.path().join("baz2.txt");

        let mut foo_file = File::create(foo_file_path).unwrap();
        let mut bar_file = File::create(bar_file_path).unwrap();
        let baz_file = File::create(baz_file_path).unwrap();

        writeln!(foo_file, "foo").unwrap();
        writeln!(bar_file, "bar").unwrap();
        // Close all file handles, so the tempdir gets deleted when it goes
        // out of scope
        drop(foo_file);
        drop(bar_file);
        drop(baz_file);

        do_full_scan(server_pool.clone(), &server_dir.path().to_path_buf(), 1)
            .await
            .unwrap();

        assert_eq!(get_all_files(&*server_pool).await.len(), 3);

        let client_config = Arc::new(Mutex::new(Config {
            paths: vec![WatchedFolder::new_full(1, client_dir.path())],
            peers: vec![Peer {
                ip: [127, 0, 0, 1].into(),
                folders: vec![1],
            }],
        }));

        let server_config = Arc::new(Mutex::new(Config {
            paths: vec![WatchedFolder::new_full(1, server_dir.path())],
            peers: vec![Peer {
                ip: [127, 0, 0, 1].into(),
                folders: vec![1],
            }],
        }));

        let (tx_client_sync_cmd, mut rx_client) = mpsc::channel(1);
        let (tx_server_sync_cmd, mut rx_server) = mpsc::channel(1);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(_) = rx_client.recv() => {},
                    Some(_) = rx_server.recv() => {}
                }
            }
        });
        // Client
        let client_pool_handle = client_pool.clone();
        tokio::spawn(async move {
            try_connect(client_pool_handle, client_config, tx_client_sync_cmd, 1236).await;
        });

        wait_incoming_intern(
            server_pool.clone(),
            server_config,
            tx_server_sync_cmd,
            1236,
            true,
        )
        .await;

        let server_files = get_all_files(&*server_pool).await;
        assert_eq!(server_files.len(), 5);
        assert!(server_files.iter().any(|f| f.path.ends_with("foo.txt")));
        assert!(server_files.iter().any(|f| f.path.ends_with("bar.txt")));
        assert!(server_files.iter().any(|f| f.path.ends_with("foo2.txt")));
        assert!(server_files.iter().any(|f| f.path.ends_with("bar2.txt")));
        assert!(server_files.iter().any(|f| f.path.ends_with("baz2.txt")));
        assert!(server_files
            .iter()
            .filter(|f| f.path.ends_with("foo.txt"))
            .all(|f| f.local_hash.is_none()));
        assert!(server_files
            .iter()
            .filter(|f| f.path.ends_with("bar.txt"))
            .all(|f| f.local_last_modified.is_none()));
        assert!(server_files
            .iter()
            .filter(|f| f.path.ends_with("foo2.txt"))
            .all(|f| f.local_last_modified.is_some()));

        assert!(server_files
            .iter()
            .filter(|f| f.path.ends_with("bar2.txt"))
            .all(|f| f.local_last_modified.is_some()));

        assert!(server_files
            .iter()
            .filter(|f| f.path.ends_with("baz2.txt"))
            .all(|f| f.local_last_modified.is_some()));

        let client_files = get_all_files(&*client_pool).await;
        assert_eq!(client_files.len(), 5);
        assert!(client_files.iter().any(|f| f.path.ends_with("foo.txt")));
        assert!(client_files.iter().any(|f| f.path.ends_with("bar.txt")));
        assert!(client_files.iter().any(|f| f.path.ends_with("foo2.txt")));
        assert!(client_files.iter().any(|f| f.path.ends_with("bar2.txt")));
        assert!(client_files.iter().any(|f| f.path.ends_with("baz2.txt")));
    }

    #[tokio::test]
    async fn test_file_sync() {
        init();
        let (client_pool, server_pool) = create_two_dbs().await;
        // Client setup
        let client_dir = create_test_dir();
        do_full_scan(client_pool.clone(), &client_dir.path().to_path_buf(), 1)
            .await
            .unwrap();

        let client_config = Arc::new(Mutex::new(Config {
            paths: vec![WatchedFolder::new_full(1, client_dir.path())],
            peers: vec![Peer {
                ip: [127, 0, 0, 1].into(),
                folders: vec![1],
            }],
        }));

        // Server setup
        let server_dir = tempdir().unwrap();
        do_full_scan(server_pool.clone(), &server_dir.path().to_path_buf(), 1)
            .await
            .unwrap();

        let server_config = Arc::new(Mutex::new(Config {
            paths: vec![WatchedFolder::new_full(1, server_dir.path())],
            peers: vec![Peer {
                ip: [127, 0, 0, 1].into(),
                folders: vec![1],
            }],
        }));

        let (tx_client_sync_cmd, rx_client) = mpsc::channel(1);
        let (tx_server_sync_cmd, rx_server) = mpsc::channel(1);

        // Start trying to sync our files
        let client_pool_handle = client_pool.clone();
        let client_config_handle = client_config.clone();
        tokio::spawn(async move {
            sync_files(client_pool_handle, client_config_handle, rx_client, 1238).await;
        });

        let server_pool_handle = server_pool.clone();
        let server_config_handle = server_config.clone();
        let server_sync_files = tokio::spawn(async move {
            sync_files_internal(
                server_pool_handle,
                server_config_handle,
                rx_server,
                1239,
                true,
            )
            .await;
        });

        // He also wants to listen for file syncs
        let client_config_handle = client_config.clone();
        tokio::spawn(async move {
            listen_file_sync(client_config_handle, 1239).await;
        });

        // So does the server
        let server_config_handle = server_config.clone();
        tokio::spawn(async move {
            listen_file_sync(server_config_handle, 1238).await;
        });

        // Server starts listening to the client for database sync
        tokio::spawn(async move {
            wait_incoming(server_pool.clone(), server_config, tx_server_sync_cmd, 1237).await;
        });

        // Run Client
        let client_pool_handle = client_pool.clone();
        let client_config_handle = client_config.clone();
        tokio::spawn(async move {
            log::info!("Trying to connect...");
            try_connect(
                client_pool_handle,
                client_config_handle,
                tx_client_sync_cmd,
                1237,
            )
            .await;
        });

        // Wait until we synced all files
        server_sync_files.await.unwrap();

        let files = WalkDir::new(server_dir.path())
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.metadata().map(|m| m.is_file()).unwrap_or(false))
            .collect::<Vec<_>>();

        assert_eq!(files.len(), 2);

        let foo_file_path = server_dir.path().join("foo.txt");
        let bar_file_path = server_dir.path().join("bar.txt");

        let mut foo_file = File::open(foo_file_path).unwrap();
        let mut bar_file = File::open(bar_file_path).unwrap();

        let mut foo_content = String::new();
        let mut bar_content = String::new();

        foo_file.read_to_string(&mut foo_content).unwrap();
        bar_file.read_to_string(&mut bar_content).unwrap();

        drop(foo_file);
        drop(bar_file);

        assert_eq!(foo_content, "foo\n");
        assert_eq!(bar_content, "bar\n");
    }
}
