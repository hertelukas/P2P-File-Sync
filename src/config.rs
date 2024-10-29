use dirs;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::PathBuf;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::types::Peer;
use crate::types::WatchedFolder;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to read file {path:?}")]
    ReadError {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("No config directory found.")]
    NoConfigDir,
    #[error(transparent)]
    ParseError(#[from] toml::de::Error),
    #[error(transparent)]
    SerializeError(#[from] toml::ser::Error),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Config {
    paths: Vec<WatchedFolder>,
    peers: Vec<Peer>,
}

impl Config {
    /// Loads the configuration from disk.
    /// If none exists, creates the default one
    pub async fn get() -> Result<Self, Error> {
        match Self::load().await {
            Ok(config) => {
                log::info!("Using existing config.");
                Ok(config)
            }
            Err(e) => match e {
                Error::ReadError {
                    path: _,
                    ref source,
                } => {
                    if source.kind() == std::io::ErrorKind::NotFound {
                        Self::create_default().await
                    } else {
                        Err(e)
                    }
                }
                _ => Err(e),
            },
        }
    }

    /// Loads a configuration file from disk.
    async fn load() -> Result<Self, Error> {
        let path = Self::config_path()?;
        let reader = tokio::fs::File::open(path.clone())
            .await
            .map_err(|source| Error::ReadError { path, source })?;
        Self::read(reader).await
    }

    async fn read<Reader>(mut reader: Reader) -> Result<Self, Error>
    where
        Reader: AsyncRead + Unpin,
    {
        let mut config_data = String::new();

        // We do not care how much we read - we just need to pass
        let _ = reader.read_to_string(&mut config_data).await;
        toml::from_str(&config_data).map_err(Error::from)
    }

    /// Stores itself in the config directory/p2p/config.toml
    async fn store(&self) -> Result<(), Error> {
        let path = Self::config_path()?;

        // TODO maybe handle unwrap here
        tokio::fs::create_dir_all(&path.parent().unwrap())
            .await
            .map_err(Error::from)?;

        let writer = tokio::fs::File::open(path).await.map_err(Error::from)?;
        Self::write(&self, writer).await
    }

    async fn write<Writer>(&self, mut writer: Writer) -> Result<(), Error>
    where
        Writer: AsyncWrite + Unpin,
    {
        let content = toml::to_string_pretty(self).map_err(Error::from)?;
        let content = format!(
            "# File automaticaly generated by p2p_file_sync
# Do not edit manually, unless you know what you are doing!\n\n{}",
            content
        );

        writer
            .write_all(content.as_bytes())
            .await
            .map_err(Error::from)?;

        Ok(())
    }

    /// Creates a default config, stores it and returns it
    /// TODO this currently just overrides existing configs
    async fn create_default() -> Result<Self, Error> {
        log::info!("Creating default config...");
        let mut paths: Vec<WatchedFolder> = vec![];
        if let Some(docs) = dirs::document_dir() {
            paths.push(WatchedFolder::new(docs));
        };

        let config = Config {
            paths,
            peers: vec![],
        };

        config.store().await?;

        Ok(config)
    }

    fn config_path() -> Result<PathBuf, Error> {
        if let Some(config_path) = dirs::config_dir() {
            Ok(config_path.join("p2p").join("config.toml"))
        } else {
            Err(Error::NoConfigDir)
        }
    }

    pub fn paths(&self) -> &Vec<WatchedFolder> {
        &self.paths
    }

    /// Returns the id's of the folders, shared with the peer
    pub fn shared_folders<T>(&self, ip: T) -> Option<Vec<u32>>
    where
        T: Into<IpAddr>,
    {
        let ip = ip.into();
        for peer in &self.peers {
            if peer.ip == ip {
                return Some(peer.folders().clone());
            }
        }
        None
    }

    pub fn peer_ips(&self) -> Vec<IpAddr> {
        let mut res = vec![];
        for peer in &self.peers {
            res.push(peer.ip.clone());
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_emtpy() {
        let conf = Config {
            paths: vec![],
            peers: vec![],
        };

        let reader = tokio_test::io::Builder::new()
            .read(b"paths = []\npeers = []\n")
            .build();
        let c = Config::read(reader).await.unwrap();

        assert_eq!(c, conf)
    }

    #[tokio::test]
    async fn read_normal() {
        let folder = WatchedFolder::new("/tmp");
        let id = folder.id();
        let conf = Config {
            paths: vec![folder],
            peers: vec![],
        };

        let bytes = format!(
            "peers = []
[[paths]]
id = {id}
path=\"/tmp\""
        )
        .into_bytes();

        let reader = tokio_test::io::Builder::new()
            .read(bytes.as_slice())
            .build();

        let c = Config::read(reader).await.unwrap();

        assert_eq!(c, conf);
    }

    #[tokio::test]
    async fn write_read() {
        use tokio::io::BufWriter;
        let folder1 = WatchedFolder::new("/tmp");
        let folder2 = WatchedFolder::new("/tmp/foo");

        let peer = Peer::new([127, 0, 0, 1]);

        let conf = Config {
            paths: vec![folder1, folder2],
            peers: vec![peer],
        };

        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // Write our config into the buffer (instead of a file)
        conf.write(&mut writer)
            .await
            .expect("Failed to write config");

        // Write the content of the buffer into our mock reader
        let written = writer.buffer();
        let reader = tokio_test::io::Builder::new().read(written).build();

        let c = Config::read(reader)
            .await
            .expect("Failed to read written config");

        assert_eq!(c, conf);
    }

    #[test]
    fn test_shared_folders() {
        let mut p1 = Peer::new([127, 0, 0, 1]);
        let mut p2 = Peer::new([192, 168, 0, 1]);

        p1.share_folder(1);
        p1.share_folder(3);
        p2.share_folder(2);

        let conf = Config {
            paths: vec![],
            peers: vec![p1, p2],
        };

        assert_eq!(conf.shared_folders([127, 0, 0, 1]), vec![1, 3].into());
        assert_eq!(conf.shared_folders([192, 168, 0, 1]), vec![2].into());
    }
}
