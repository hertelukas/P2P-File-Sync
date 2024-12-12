//! Module responsible for syncing database state and file state
use std::{
    fs,
    io::Write,
    net::IpAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use bytes::Bytes;
use futures::StreamExt;
use sqlx::query_as;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::{Receiver, Sender},
};

use crate::{config::Config, connection::Connection, frame::Frame, types::File};

type MutexConf = Arc<Mutex<Config>>;

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
                    handle_outgoing_connection(stream, config_handle, pool_handle, tx_handle).await;
                });
            }
        }
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}

async fn handle_outgoing_connection(
    stream: TcpStream,
    config: MutexConf,
    pool: Arc<sqlx::SqlitePool>,
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
    // First, we send all our db states,
    // which should not be different to normal introduced updates
    connection
        .write_frame(&Frame::InitiateDbSync)
        .await
        .expect("Could not send frame to initate db sync");
    for folder in folders {
        send_db_state(&mut connection, folder, config.clone(), pool.clone()).await;
        connection.write_frame(&Frame::Done).await.unwrap();
    }
    connection.write_frame(&Frame::Done).await.unwrap();

    // Second, we request the db state of our peer
    connection
        .write_frame(&Frame::RequestDbSync)
        .await
        .expect("Could not send frame to request db sync");
    receive_db_state(&mut connection, config.clone(), pool.clone()).await;
    connection.write_frame(&Frame::Done).await.unwrap();
    tx_sync_cmd.send(()).await.unwrap();
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

    log::info!("Listening for sync on {:?}", listener.local_addr());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let config = config.clone();
                let pool = pool.clone();
                let tx_handle = tx_sync_cmd.clone();

                let t = tokio::spawn(async move {
                    handle_incoming_connection(stream, config, pool, tx_handle).await;
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
async fn handle_incoming_connection(
    stream: TcpStream,
    config: MutexConf,
    pool: Arc<sqlx::SqlitePool>,
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

    // Check if peer wants file, wants to update our database or requests database update
    while let Some(frame) = connection.read_frame().await.expect("Failed to read frame") {
        log::info!(
            "New connection from {:?}, wants to {:?}",
            connection.get_peer_ip(),
            frame
        );
        match frame {
            // Peer wants to send us update
            Frame::InitiateDbSync => {
                receive_db_state(&mut connection, config.clone(), pool.clone()).await;
            }
            // We want to send update
            Frame::RequestDbSync => {
                for folder_id in &folders {
                    send_db_state(&mut connection, *folder_id, config.clone(), pool.clone()).await;
                    connection.write_frame(&Frame::Done).await.unwrap();
                }
                connection.write_frame(&Frame::Done).await.unwrap();
            }
            Frame::RequestFile { folder_id, path } => {
                let base_path = {
                    let lock = config.lock().unwrap();
                    lock.get_path(folder_id)
                };
                if let Some(base_path) = base_path {
                    let path = File::get_full_path(&base_path.to_string_lossy(), &path);

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
            Frame::Done => {
                log::info!(
                    "Exchange done with {connection:?}, dropping connection, let us sync files"
                );
                // Notify our file syncer that he might have to sync now
                tx_sync_cmd.send(()).await.unwrap();
                return;
            }
            _ => {
                log::warn!("Did not expect {frame:?} as first message on an incoming connection");
                return;
            }
        }
    }
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
                    connection.write_frame(&Frame::Yes).await.unwrap();
                    while let Some(frame) = connection.read_frame().await.unwrap() {
                        match frame {
                            Frame::InitiatorGlobal {
                                global_hash,
                                global_last_modified,
                                global_peer,
                                path,
                            } => {
                                insert_or_update_file(
                                    connection,
                                    config.clone(),
                                    pool.clone(),
                                    global_hash,
                                    global_last_modified,
                                    global_peer,
                                    path,
                                    folder_id,
                                )
                                .await;
                                // And wait for the next file
                                connection.write_frame(&Frame::Yes).await.unwrap();
                            }
                            Frame::Done => {
                                log::info!("Received information for all files in {folder_id}");
                                break;
                            }
                            _ => log::warn!(
                                "Unexpected packet received while waiting for new file information"
                            ),
                        }
                    }
                } else {
                    // If we do not want this folder information, we expect a new DbSync frame
                    let _ = connection.write_frame(&Frame::No).await;
                }
            }
            Frame::Done => {
                log::info!(
                    "Received all folder information from peer {:?}",
                    connection.get_peer_ip()
                );
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

                            // Potentially create directory
                            if let Some(parent) = path.parent() {
                                std::fs::create_dir_all(parent).unwrap();
                            }

                            let mut new_file = std::fs::File::create(path.clone()).unwrap();
                            new_file.write_all(&data).unwrap();

                            // Update time, so it does not reflect the time when we synced
                            new_file
                                .set_modified(File::unix_time_as_system(file.global_last_modified))
                                .unwrap();

                            log::info!("Scanning our newly received file at {:?}", path);
                            // And update the database
                            crate::scan::scan_file(
                                pool.clone(),
                                &path,
                                file.folder_id.try_into().unwrap(),
                            )
                            .await
                            .unwrap();
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
            log::info!("This was a one-shot sync, so we are done!");
            return;
        }
        log::info!("File sync done");
    }
}

async fn insert_or_update_file(
    connection: &mut Connection,
    config: MutexConf,
    pool: Arc<sqlx::SqlitePool>,
    global_hash: Bytes,
    global_last_modified: i64,
    global_peer: String,
    path: String,
    folder_id: u32,
) {
    let base_path = {
        let lock = config.lock().unwrap();
        lock.get_path(folder_id)
    };
    if let Some(base_path) = base_path {
        let full_path = File::get_full_path(&base_path.to_string_lossy(), &path)
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
            if res.global_hash != global_hash && res.global_last_modified < global_last_modified {
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
            // Can be that we in the meantime track it, so ignore insert fails
            let _ = sqlx::query!(
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
            .await;
        }
    } else {
        // Should never happen
        log::warn!("Tried to sync non-exisitng folder {folder_id}");
    }
}

/// Announce to `to` that `file` has changed
pub async fn announce_change(
    to: IpAddr,
    file: File,
    relative_path: String,
    folder_id: u32,
    port: u16,
) {
    log::info!("File {:?} has changed!", file.path);
    if let Ok(stream) = TcpStream::connect((to, port)).await {
        let mut connection = Connection::new(stream);

        if let Err(e) = connection.write_frame(&Frame::InitiateDbSync).await {
            log::warn!("Failed to send update to {to}: {e}");
            return;
        }

        if let Err(e) = connection.write_frame(&Frame::DbSync { folder_id }).await {
            log::warn!("Failed to send update to {to}: {e}");
            return;
        }

        if let Ok(Some(Frame::Yes)) = connection.read_frame().await {
            if let Err(e) = connection
                .write_frame(&Frame::InitiatorGlobal {
                    global_hash: file.global_hash.into(),
                    global_last_modified: file.global_last_modified,
                    global_peer: file.global_peer,
                    path: relative_path,
                })
                .await
            {
                log::warn!("Failed to send file data to {to}: {e}");
            }
        } else {
            log::warn!("Peer {to} does not seem to be interested in file update")
        }
        connection.write_frame(&Frame::Done).await.unwrap();
        connection.write_frame(&Frame::Done).await.unwrap();
        connection.write_frame(&Frame::Done).await.unwrap();
    } else {
        log::warn!("Could not notify peer about change!");
    }
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
    use walkdir::WalkDir;

    use crate::{scan::scan_folder, Peer, WatchedFolder};

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

    #[tokio::test]
    async fn test_sync_db() {
        init();
        let (client_pool, server_pool) = create_two_dbs().await;

        let client_dir = create_test_dir();
        scan_folder(client_pool.clone(), &client_dir.path().to_path_buf(), 1)
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
        scan_folder(client_pool.clone(), &client_dir.path().to_path_buf(), 1)
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
        scan_folder(client_pool.clone(), &client_dir.path().to_path_buf(), 1)
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

        scan_folder(server_pool.clone(), &server_dir.path().to_path_buf(), 1)
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
        scan_folder(client_pool.clone(), &client_dir.path().to_path_buf(), 1)
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
        scan_folder(server_pool.clone(), &server_dir.path().to_path_buf(), 1)
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

        let server_port = 1237;
        let client_port = 1238;
        // Start trying to sync our files
        let client_pool_handle = client_pool.clone();
        let client_config_handle = client_config.clone();
        tokio::spawn(async move {
            sync_files(
                client_pool_handle,
                client_config_handle,
                rx_client,
                server_port,
            )
            .await;
        });

        let server_pool_handle = server_pool.clone();
        let server_config_handle = server_config.clone();
        let server_sync_files = tokio::spawn(async move {
            sync_files_internal(
                server_pool_handle,
                server_config_handle,
                rx_server,
                client_port,
                true,
            )
            .await;
        });

        // Server starts listening to the client for database sync
        tokio::spawn(async move {
            wait_incoming(
                server_pool.clone(),
                server_config,
                tx_server_sync_cmd,
                server_port,
            )
            .await;
        });

        // same for client, as he has to be able to react to file
        // requests from the server
        let client_pool_handle = client_pool.clone();
        let client_config_handle = client_config.clone();
        let tx_client_handle = tx_client_sync_cmd.clone();
        tokio::spawn(async move {
            wait_incoming(
                client_pool_handle,
                client_config_handle,
                tx_client_handle,
                client_port,
            )
            .await;
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
                server_port,
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
