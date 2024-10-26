use ring::digest::Digest;
use sqlx::{migrate::MigrateDatabase, sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::{fs, path::PathBuf};

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

fn get_database_path() -> Result<PathBuf, Error> {
    let mut path = dirs::data_dir().ok_or(Error::NoDataDir)?;
    path.push("p2p_file_sync");
    fs::create_dir_all(&path).map_err(Error::from)?;
    path.push("db.sqlite");
    Ok(path)
}
