use std::path::PathBuf;

use walkdir::WalkDir;

#[derive(Debug, thiserror::Error)]
pub enum Error {}

pub fn sync(path: &PathBuf) -> Result<(), Error> {
    Ok(())
}

pub async fn do_full_scan(path: &PathBuf) -> Result<(), Error> {
    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        log::info!("Scanned: {entry:?}");
    }

    Ok(())
}
