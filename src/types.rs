use std::{
    fmt,
    net::IpAddr,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
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
}

impl fmt::Display for WatchedFolder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.id, self.path.to_str().unwrap())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Peer {
    ip: IpAddr,
    folders: Vec<u32>,
}
