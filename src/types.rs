use std::{
    fmt,
    net::IpAddr,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};
use walkdir::DirEntry;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct WatchedFolder {
    id: u32,
    path: PathBuf,
}

impl WatchedFolder {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        WatchedFolder {
            id: rand::random(),
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
        write!(f, "{}: {}", self.id, self.path.to_str().unwrap())
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Peer {
    ip: IpAddr,
    folders: Vec<u32>,
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
}

#[derive(sqlx::FromRow)]
pub struct File {
    pub path: String,
    pub local_hash: String,
    pub global_hash: String,
    pub last_modified: i64,
}

impl File {
    pub fn new(hash: String, entry: &DirEntry) -> Self {
        let time = Self::get_last_modified_as_unix(entry);
        File {
            path: entry.path().to_string_lossy().to_string(),
            local_hash: hash.clone(),
            global_hash: hash,
            last_modified: time,
        }
    }

    pub fn get_last_modified_as_unix(entry: &DirEntry) -> i64 {
        entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified_time| modified_time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0)
    }
}
