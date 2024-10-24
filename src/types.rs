use std::{
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Peer {
    ip: IpAddr,
    folders: Vec<u32>,
}
