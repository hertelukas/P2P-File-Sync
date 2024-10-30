use sqlx::{migrate::MigrateDatabase, sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::types::File;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("No data directory found")]
    NoDataDir,
    #[error(transparent)]
    DatabaseError(#[from] sqlx::Error),
    #[error(transparent)]
    MigrateError(#[from] sqlx::migrate::MigrateError),
}

pub async fn setup() -> Result<Pool<Sqlite>, Error> {
    let db_url = format!("sqlite://{}", get_database_path()?.to_string_lossy());

    // Possibly create new database
    if !Sqlite::database_exists(&db_url).await.unwrap_or(false) {
        log::info!("Creating database at {db_url}");
        Sqlite::create_database(&db_url).await?;
    }

    // Create SQLite connection pool
    let pool = SqlitePoolOptions::new()
        .connect(&db_url)
        .await
        .map_err(Error::from)?;

    sqlx::migrate!().run(&pool).await?;

    Ok(pool)
}

pub async fn is_tracked(pool: &sqlx::SqlitePool, path: &Path) -> Result<bool, Error> {
    let s = path.to_string_lossy().to_string();
    let res = sqlx::query!(
        r#"
SELECT COUNT(*) as count
FROM files
WHERE path = ?
"#,
        s
    )
    .fetch_one(pool)
    .await
    .map_err(Error::from)?;

    Ok(res.count > 0)
}

/// Check if `modified` is newer than the last local version we have tracked at `path`
/// If we only track the file globally - and do not have a local version yet, this
/// will return true.
pub async fn is_newer(pool: &sqlx::SqlitePool, modified: i64, path: &Path) -> Result<bool, Error> {
    let s = path.to_string_lossy().to_string();
    let res = sqlx::query!(
        r#"
SELECT local_last_modified
FROM files
WHERE path = ?
"#,
        s
    )
    .fetch_one(pool)
    .await
    .map_err(Error::from)?;

    if let Some(local_last_modified) = res.local_last_modified {
        Ok(modified > local_last_modified)
    } else {
        Ok(true)
    }
}

/// Updates the state of the tracked file at `path`.
/// If the modified date is more recent than the last local stored
/// local modified date, we update the local_hash.
/// If it is even more recent than the global version, we also update
/// the global_last_modified and the global_hash to this version, while
/// setting the global_peer to ourselves.
pub async fn update_if_newer(
    pool: &sqlx::SqlitePool,
    modified: i64,
    hash: Vec<u8>,
    path: &Path,
) -> Result<(), Error> {
    let s = path.to_string_lossy().to_string();
    sqlx::query!(
        r#"
UPDATE files
SET local_hash = ?, local_last_modified = ?
WHERE path = ? AND ? > local_last_modified
"#,
        hash,
        modified,
        s,
        modified
    )
    .execute(pool)
    .await?;

    sqlx::query!(
        r#"
UPDATE files
SET global_hash = ?, global_last_modified = ?, global_peer = "0"
WHERE path = ? AND ? > global_last_modified
"#,
        hash,
        modified,
        s,
        modified
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert(pool: &sqlx::SqlitePool, file: File) -> Result<(), Error> {
    // Insert the record into the files table
    sqlx::query!(
        r#"
INSERT INTO files (path, local_hash, local_last_modified, global_hash, global_last_modified, global_peer)
VALUES (?, ?, ?, ?, ?, ?)
        "#,
        file.path,
        file.local_hash,
        file.local_last_modified,
        file.global_hash,
        file.global_last_modified,
        file.global_peer
    )
    .execute(pool)
    .await?;

    Ok(())
}

fn get_database_path() -> Result<PathBuf, Error> {
    let mut path = dirs::data_dir().ok_or(Error::NoDataDir)?;
    path.push("p2p_file_sync");
    fs::create_dir_all(&path).map_err(Error::from)?;
    path.push("db.sqlite");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;

    use super::*;

    async fn fill_db(pool: &SqlitePool) {
        sqlx::query(
            r#"
INSERT INTO files (path, local_hash, local_last_modified, global_hash, global_last_modified, global_peer)
VALUES ("/old", "aa", 12, "bb", 12, "0"),
("/new", "aa", 100, "aa", 100, "0")
"#,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    // This probably should be a real function at some point
    async fn get_file(pool: &SqlitePool, path: &Path) -> File {
        let s = path.to_string_lossy().to_string();
        sqlx::query_as!(
            File,
            r#"
SELECT *
FROM files
WHERE path = ?
"#,
            s
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test]
    async fn test_tracked(pool: SqlitePool) {
        fill_db(&pool).await;

        // Check if tracked file is tracked
        assert!(is_tracked(&pool, &Path::new("/old")).await.unwrap());
    }

    #[sqlx::test]
    async fn test_not_track(pool: SqlitePool) {
        fill_db(&pool).await;

        // Check that new file is marked as untracked
        assert!(!is_tracked(&pool, &Path::new("/does-not-exist"))
            .await
            .unwrap());
    }

    #[sqlx::test]
    async fn test_is_newer(pool: SqlitePool) {
        fill_db(&pool).await;

        assert!(is_newer(&pool, 100, &Path::new("/old")).await.unwrap());
    }

    #[sqlx::test]
    async fn test_modified_same(pool: SqlitePool) {
        fill_db(&pool).await;

        assert!(!is_newer(&pool, 100, &Path::new("/new")).await.unwrap());
    }

    #[sqlx::test]
    async fn test_modified_older(pool: SqlitePool) {
        fill_db(&pool).await;

        assert!(!is_newer(&pool, 99, &Path::new("/new")).await.unwrap());
    }

    #[sqlx::test]
    async fn test_modified_negative(pool: SqlitePool) {
        fill_db(&pool).await;

        assert!(!is_newer(&pool, -1000, &Path::new("/old")).await.unwrap());
    }

    #[sqlx::test]
    async fn test_insert(pool: SqlitePool) {
        fill_db(&pool).await;

        let f = File {
            path: "/insert".to_owned(),
            local_hash: Some("aa".to_owned().into()),
            local_last_modified: Some(0),
            global_hash: "bb".to_owned().into(),
            global_last_modified: 0,
            global_peer: "0".to_owned(),
        };

        insert(&pool, f).await.unwrap();

        assert!(is_tracked(&pool, &Path::new("/insert")).await.unwrap());
    }

    #[sqlx::test]
    #[should_panic]
    async fn test_insert_duplicate(pool: SqlitePool) {
        fill_db(&pool).await;

        let f = File {
            path: "/new".to_owned(),
            local_hash: Some("aa".to_owned().into()),
            local_last_modified: Some(0),
            global_hash: "bb".to_owned().into(),
            global_last_modified: 0,
            global_peer: "0".to_owned(),
        };

        insert(&pool, f).await.unwrap();
    }

    #[sqlx::test]
    async fn test_set_local_hash(pool: SqlitePool) {
        fill_db(&pool).await;

        update_if_newer(&pool, 101, "xx".to_owned().into(), &Path::new("/new"))
            .await
            .unwrap();

        let f = get_file(&pool, &Path::new("/new")).await;
        assert_eq!(f.local_hash, Some(b"xx".to_vec()));
        assert_eq!(f.local_last_modified, Some(101));
    }

    #[sqlx::test]
    async fn test_do_not_udpate_hash_if_old(pool: SqlitePool) {
        fill_db(&pool).await;

        update_if_newer(&pool, 99, "xx".to_owned().into(), &Path::new("/new"))
            .await
            .unwrap();

        let f = get_file(&pool, &Path::new("/new")).await;
        assert_eq!(f.local_hash, Some(b"aa".to_vec()));
        assert_eq!(f.local_last_modified, Some(100));
    }

    #[sqlx::test]
    async fn test_do_not_udpate_hash_if_same(pool: SqlitePool) {
        fill_db(&pool).await;

        update_if_newer(&pool, 100, "xx".to_owned().into(), &Path::new("/new"))
            .await
            .unwrap();

        let f = get_file(&pool, &Path::new("/new")).await;
        assert_eq!(f.local_hash, Some(b"aa".to_vec()));
        assert_eq!(f.local_last_modified, Some(100));
    }
}
