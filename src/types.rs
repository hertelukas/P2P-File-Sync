use std::net::IpAddr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct WatchedFolder {
    id: u32,
    path: String
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Peer {
    ip: IpAddr,
    folders: Vec<u32>
}
