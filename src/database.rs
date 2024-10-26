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
