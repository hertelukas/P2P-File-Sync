///! Module which allows to encode the current file state to an SQL database
use std::{fs, path::PathBuf, sync::Arc, time::SystemTime};

use sha2::{Digest, Sha256};
use tokio::fs::read;
use walkdir::WalkDir;

use crate::{
    database::{insert, is_newer, is_tracked, update_if_newer},
    types::File,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    IoError(#[from] tokio::io::Error),
    #[error(transparent)]
    DatabaseError(#[from] crate::database::Error),
    #[error("Could not scan file information")]
    ScanError,
}

/// Helper function which returns the Sha256 of `data`
fn hash_data(data: Vec<u8>) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Updates the database by recursively iterating over all files in the path.
/// This is done by following these steps:
/// 1. Check if the file is tracked: If not, insert and done.
/// 2. Check if the file has a newer modified date. If not, done.
/// 3. Calculate the file hash and update
pub async fn scan_folder(
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
        if !is_tracked(&pool, &entry.path(), folder_id).await? {
            let content = read(entry.path()).await.map_err(Error::from)?;
            log::info!("Tracking {entry:?}");
            let time = File::get_last_modified_as_unix(&entry);
            let f = File::new(
                folder_id,
                hash_data(content),
                entry.path().to_string_lossy().to_string(),
                time,
            );
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
                folder_id,
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
                    folder_id,
                )
                .await?;
            } else {
                log::debug!("No updated needed for {entry:?}");
            }
        }
    }

    Ok(())
}

/// Updates the database for the following file
pub async fn scan_file(
    pool: Arc<sqlx::SqlitePool>,
    path: &PathBuf,
    folder_id: u32,
) -> Result<File, Error> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) => {
            // TODO this is the default when we deleted the file,
            // so we should handle it at some point
            log::info!("Could not retrieve file metadata for {path:?}: {e}");
            return Err(Error::ScanError);
        }
    };

    if !metadata.is_file() {
        log::warn!("Trying to update non-file");
        return Err(Error::ScanError);
    }

    let content = read(path).await.map_err(Error::from)?;
    let time = File::system_time_as_unix(metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));
    // Handle new file
    if !is_tracked(&pool, path, folder_id).await? {
        log::info!("Tracking {path:?}");
        let f = File::new(
            folder_id,
            hash_data(content),
            path.to_string_lossy().to_string(),
            time,
        );
        insert(&pool, f).await.map_err(Error::from)?;
    } else {
        update_if_newer(&pool, time, hash_data(content), &path, folder_id).await?;
    }

    let s = path.to_string_lossy().to_string();
    sqlx::query_as!(
        File,
        r#"
SELECT *
FROM files
WHERE path = ? AND folder_id = ?
"#,
        s,
        folder_id
    )
    .fetch_one(&*pool)
    .await
    .map_err(crate::database::Error::from)
    .map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use sqlx::{query_as, SqlitePool};
    use std::fs::File;
    use std::io::Write;
    use tempfile::{tempdir, TempDir};

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

    use super::*;
    #[sqlx::test]
    async fn test_normal_scan(pool: SqlitePool) {
        let dir = create_test_dir();
        let pool = Arc::new(pool);
        scan_folder(pool.clone(), &dir.path().to_path_buf(), 6)
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
        scan_folder(pool.clone(), &dir.path().to_path_buf(), 6)
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
        scan_folder(pool.clone(), &dir.path().to_path_buf(), 6)
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
        scan_folder(pool.clone(), &dir.path().to_path_buf(), 6)
            .await
            .unwrap();

        let files = get_all_files(&*pool).await;

        assert_eq!(files.len(), 3);
        assert!(files.into_iter().any(|f| f.path.contains("third")))
    }
}
