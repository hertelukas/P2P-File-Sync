use std::path::PathBuf;

use tokio::fs::read;
use walkdir::WalkDir;

use crate::{
    database::{insert, is_newer, is_tracked},
    types::File,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    IoError(#[from] tokio::io::Error),
    #[error(transparent)]
    DatabaseError(#[from] crate::database::Error),
}

pub fn sync(path: &PathBuf) -> Result<(), Error> {
    Ok(())
}

pub async fn do_full_scan(pool: &sqlx::SqlitePool, path: &PathBuf) -> Result<(), Error> {
    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.metadata().map(|m| m.is_file()).unwrap_or(false))
    {
        // Insert the file in our database, if untracked
        if !is_tracked(pool, &entry.path()).await? {
            let content = read(entry.path()).await.map_err(Error::from)?;
            let hash = sha256::digest(content);
            log::info!("Inserting: {entry:?}");
            let f = File::new(hash, &entry);
            insert(pool, f).await.map_err(Error::from)?;
        }
        // We are tracking the file already, check for newer version
        else {
            if is_newer(pool, File::get_last_modified_as_unix(&entry), &entry.path()).await? {
                // Check if hash changed
                todo!();
            }
            else {
                log::debug!("No updated needed for {entry:?}");
            }
        }
    }

    Ok(())
}
