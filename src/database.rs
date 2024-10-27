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

pub async fn is_newer(pool: &sqlx::SqlitePool, modified: i64, path: &Path) -> Result<bool, Error> {
    let s = path.to_string_lossy().to_string();
    let res = sqlx::query!(
        r#"
SELECT last_modified
FROM files
WHERE path = ?
"#,
        s
    )
    .fetch_one(pool)
    .await
    .map_err(Error::from)?;

    Ok(modified > res.last_modified)
}

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
SET local_hash = ?, last_modified = ?
WHERE path = ? AND ? > last_modified
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
INSERT INTO files (path, local_hash, global_hash, last_modified)
VALUES (?, ?, ?, ?)
        "#,
        file.path,
        file.local_hash,
        file.global_hash,
        file.last_modified
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
INSERT INTO files (path, local_hash, global_hash, last_modified)
VALUES ("/old", "aa", "bb", 12),
("/new", "aa", "aa", 100)
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
            local_hash: "aa".to_owned().into(),
            global_hash: "bb".to_owned().into(),
            last_modified: 0,
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
            local_hash: "aa".to_owned().into(),
            global_hash: "bb".to_owned().into(),
            last_modified: 0,
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
        assert_eq!(f.local_hash, b"xx".to_vec());
        assert_eq!(f.last_modified, 101);
    }

    #[sqlx::test]
    async fn test_do_not_udpate_hash_if_old(pool: SqlitePool) {
        fill_db(&pool).await;

        update_if_newer(&pool, 99, "xx".to_owned().into(), &Path::new("/new"))
            .await
            .unwrap();

        let f = get_file(&pool, &Path::new("/new")).await;
        assert_eq!(f.local_hash, b"aa".to_vec());
        assert_eq!(f.last_modified, 100);
    }

    #[sqlx::test]
    async fn test_do_not_udpate_hash_if_same(pool: SqlitePool) {
        fill_db(&pool).await;

        update_if_newer(&pool, 100, "xx".to_owned().into(), &Path::new("/new"))
            .await
            .unwrap();

        let f = get_file(&pool, &Path::new("/new")).await;
        assert_eq!(f.local_hash, b"aa".to_vec());
        assert_eq!(f.last_modified, 100);
    }

}
