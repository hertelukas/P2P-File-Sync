use std::{
    fmt,
    net::IpAddr,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct WatchedFolder {
    id: u32,
    pub path: PathBuf,
}

impl WatchedFolder {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        WatchedFolder {
            id: rand::random(),
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn new_full<P: AsRef<Path>>(id: u32, path: P) -> Self {
        WatchedFolder {
            id,
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn id(&self) -> u32 {
        self.id
    }
}

impl fmt::Display for WatchedFolder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#10x} : {}", self.id, self.path.to_str().unwrap())
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct Peer {
    pub ip: IpAddr,
    pub folders: Vec<u32>,
}

impl Peer {
    pub fn new<T>(ip: T) -> Self
    where
        T: Into<IpAddr>,
    {
        Peer {
            ip: ip.into(),
            folders: vec![],
        }
    }

    pub fn folders(&self) -> Vec<u32> {
        self.folders.clone()
    }

    pub fn share_folder(&mut self, id: u32) {
        self.folders.push(id);
    }
}

impl fmt::Display for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.ip)?;
        if self.folders.len() > 0 {
            writeln!(f, "")?;
        }
        for id in &self.folders {
            writeln!(f, "{:#10x}", id)?;
        }
        Ok(())
    }
}

#[derive(Debug, sqlx::FromRow, Clone)]
pub struct File {
    pub folder_id: i64,
    pub path: String,
    pub local_hash: Option<Vec<u8>>,
    pub local_last_modified: Option<i64>,
    pub global_hash: Vec<u8>,
    pub global_last_modified: i64,
    pub global_peer: String,
}

impl File {
    /// Used to create a local, not-yet tracked file
    pub fn new(folder_id: u32, hash: Vec<u8>, path: String, time: i64) -> Self {
        File {
            // This is necessary, as we use an i64 in the database
            folder_id: folder_id.into(),
            path,
            local_hash: Some(hash.clone()),
            global_hash: hash,
            local_last_modified: Some(time),
            global_last_modified: time,
            global_peer: "0".to_owned(),
        }
    }

    pub fn system_time_as_unix(modified: SystemTime) -> i64 {
        modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0)
    }

    pub fn unix_time_as_system(modified: i64) -> SystemTime {
        if modified >= 0 {
            UNIX_EPOCH + Duration::from_secs(modified as u64)
        } else {
            UNIX_EPOCH - Duration::from_secs(modified.abs() as u64)
        }
    }

    /// Turns the `full_path` at a relative path of `base` and returns `Some(relative_path)`
    /// on success. None, otherwise, e.g., when `full_path` is not a prefix of `base`.
    pub fn get_relative_path(base: &str, full_path: &str) -> Option<PathBuf> {
        let base_path = Path::new(base);
        let file_path = Path::new(full_path);

        match file_path.strip_prefix(base_path) {
            Ok(relative) => Some(relative.to_path_buf()),
            Err(_) => None,
        }
    }

    /// Appends the `relative_path` to the `base`.
    pub fn get_full_path(base: &str, relative_path: &str) -> PathBuf {
        let base_path = Path::new(base);
        let file_path = Path::new(relative_path);

        base_path.join(file_path)
    }
}
