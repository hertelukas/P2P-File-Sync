use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::{config::Config, types::WatchedFolder};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{self, Receiver, Sender};

type MutexConf = Arc<Mutex<Config>>;

#[derive(Debug)]
pub enum WatchCommand {
    Add { folder: WatchedFolder },
    Remove { folder: WatchedFolder },
}

pub async fn watch(
    config: MutexConf,
    mut cmd_rx: Receiver<WatchCommand>,
    handler_tx: Sender<PathBuf>,
) -> notify::Result<()> {
    // Channel to receive file events
    let (tx, mut rx) = mpsc::channel(1);

    // Spawn a blocking task to use notify's sync watcher in async context
    let mut watcher = {
        let event_tx = tx.clone();
        RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                let _ = event_tx.blocking_send(res);
            },
            notify::Config::default(),
        )?
    };

    for path in config.lock().unwrap().paths() {
        log::info!("Watching {:?}", &path);
        watcher.watch(path.path(), RecursiveMode::Recursive)?;
    }

    loop {
        tokio::select! {
            Some(event) = rx.recv() => match event {
                Ok(event) => {
                    for path in event.paths {
                        handler_tx.send(path).await.unwrap();
                    }
                }
                Err(error) => log::error!("Error: {error:?}"),
            },

            Some(command) = cmd_rx.recv() => match command {
                WatchCommand::Add { folder } => {
                    log::info!("Watching new folder {:?}", &folder);
                    watcher.watch(folder.path(), RecursiveMode::Recursive)?;
                },
                WatchCommand::Remove { folder } => {
                    log::info!("No longer watching {folder:?}");
                    watcher.unwatch(folder.path())?;
                },
            }
        }
    }
}
