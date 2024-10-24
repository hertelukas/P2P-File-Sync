use crate::types::WatchedFolder;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

pub fn watch(folders: &Vec<WatchedFolder>) -> notify::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();

    let mut watcher = RecommendedWatcher::new(tx, Config::default())?;

    for path in folders {
        log::info!("Watching {:?}", &path);
        watcher.watch(path.path(), RecursiveMode::Recursive)?;
    }

    for res in rx {
        match res {
            Ok(event) => log::info!("Change: {event:?}"),
            Err(error) => log::error!("Error: {error:?}"),
        }
    }

    Ok(())
}
