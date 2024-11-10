use axum::{
    extract::State,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use reqwest::StatusCode;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Sender;

use crate::{config::Config, watcher::WatchCommand, WatchedFolder};

type MutexConf = Arc<Mutex<Config>>;

#[derive(Clone)]
struct AppState {
    pub config: MutexConf,
    pub tx_watch_command: Sender<WatchCommand>,
}

pub fn app(config: MutexConf, tx_watch_command: Sender<WatchCommand>) -> Router {
    Router::new()
        .route("/", get(get_index))
        .route("/folder", post(post_folder))
        .with_state(AppState {
            config,
            tx_watch_command,
        })
}

async fn get_index(State(state): State<AppState>) -> Json<Config> {
    let config = state.config.lock().unwrap();
    Json(config.clone())
}

#[axum::debug_handler]
async fn post_folder(
    State(state): State<AppState>,
    Json(folder): Json<WatchedFolder>,
) -> impl IntoResponse {
    log::info!("Received post for {:?}", folder);

    {
        let mut config = state.config.lock().unwrap();
        // This has to be synchronously, as we cannot hold the
        // mutex over an await
        if let Err(e) = config.add_folder_sync(folder.clone(), true) {
            log::warn!("Failed to add folder {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(()));
        }
    }
    match state
        .tx_watch_command
        .send(WatchCommand::Add { folder })
        .await
    {
        Ok(_) => (),
        Err(e) => {
            // We do not want to send failure, as we have added the folder
            // to our config succesfully at this point
            log::warn!("Could not notify file watcher of change: {e}");
        }
    }
    (StatusCode::CREATED, Json(()))
}
