use axum::{
    extract::State,
    response::IntoResponse,
    routing::{delete, get, post, put},
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
    store: bool, // So we can skip storing in tests
}

pub fn app(config: MutexConf, tx_watch_command: Sender<WatchCommand>, store: bool) -> Router {
    Router::new()
        .route("/", get(get_index))
        .route("/folder", post(post_folder))
        .route("/folder", delete(delete_folder))
        .route("/folder", put(put_folder))
        .with_state(AppState {
            config,
            tx_watch_command,
            store,
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
        if let Err(e) = config.add_folder_sync(folder.clone(), state.store) {
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

#[axum::debug_handler]
async fn delete_folder(
    State(state): State<AppState>,
    Json(folder): Json<WatchedFolder>,
) -> impl IntoResponse {
    log::info!("Received delete for {:?}", folder);

    {
        let mut config = state.config.lock().unwrap();

        if let Err(e) = config.delete_folder_sync(folder.id, state.store) {
            log::warn!("Failed to delete folder {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(()));
        }
    }

    match state
        .tx_watch_command
        .send(WatchCommand::Remove { folder })
        .await
    {
        Ok(_) => (),
        Err(e) => log::warn!("Could not notify file watcher of change: {e}"),
    }

    (StatusCode::OK, Json(()))
}

#[axum::debug_handler]
async fn put_folder(
    State(state): State<AppState>,
    Json(folder): Json<WatchedFolder>,
) -> impl IntoResponse {
    log::info!("Received put for {:?}", folder);

    if let Some(old_folder) = {
        let mut config = state.config.lock().unwrap();

        match config.update_folder_sync(folder.clone(), state.store) {
            Ok(old) => old,
            Err(e) => {
                log::warn!("Failed to update folder {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(()));
            }
        }
    } {
        match state
            .tx_watch_command
            .send(WatchCommand::Remove { folder: old_folder })
            .await
        {
            Ok(_) => match state
                .tx_watch_command
                .send(WatchCommand::Add { folder })
                .await
            {
                Ok(_) => (),
                Err(e) => log::warn!("Could not notify file watcher of change: {e}"),
            },

            Err(e) => log::warn!("Could not nofiy file watcher of change: {e}"),
        }
    }

    (StatusCode::OK, Json(()))
}

#[cfg(test)]
mod tests {
    use std::usize;

    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use serde_json::{from_slice, to_string};
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn test_index() {
        let config = Config {
            paths: vec![
                WatchedFolder {
                    id: 1,
                    path: "foo".into(),
                },
                WatchedFolder {
                    id: 2,
                    path: "bar".into(),
                },
            ],
            peers: vec![],
        };
        let (tx, _) = mpsc::channel(1);

        let app = app(Arc::new(Mutex::new(config.clone())), tx, false);

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let fetched_config: Config =
            from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();

        assert_eq!(config, fetched_config);
    }

    #[tokio::test]
    async fn test_delete() {
        let config = Arc::new(Mutex::new(Config {
            paths: vec![
                WatchedFolder {
                    id: 1,
                    path: "foo".into(),
                },
                WatchedFolder {
                    id: 2,
                    path: "bar".into(),
                },
            ],
            peers: vec![],
        }));
        let (tx, _) = mpsc::channel(1);

        let app = app(config.clone(), tx, false);

        let folder_to_delete = WatchedFolder {
            id: 1,
            path: "foo".into(),
        };

        let json_body = to_string(&folder_to_delete).unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/folder")
                    .header("Content-Type", "application/json")
                    .body(Body::from(json_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        assert_eq!(config.lock().unwrap().paths.len(), 1);
        assert_eq!(config.lock().unwrap().paths[0].id, 2);
    }

    #[tokio::test]
    async fn test_put() {
        let config = Arc::new(Mutex::new(Config {
            paths: vec![
                WatchedFolder {
                    id: 1,
                    path: "foo".into(),
                },
                WatchedFolder {
                    id: 2,
                    path: "bar".into(),
                },
            ],
            peers: vec![],
        }));
        let (tx, _) = mpsc::channel(1);

        let app = app(config.clone(), tx, false);

        let folder_to_update = WatchedFolder {
            id: 1,
            path: "foobar".into(),
        };

        let json_body = to_string(&folder_to_update).unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/folder")
                    .header("Content-Type", "application/json")
                    .body(Body::from(json_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        assert_eq!(config.lock().unwrap().paths.len(), 2);
        assert_eq!(config.lock().unwrap().paths[0].id, 1);
        assert_eq!(
            config.lock().unwrap().paths[0].path.to_string_lossy(),
            "foobar"
        );
    }

    #[tokio::test]
    async fn test_post() {
        let config = Arc::new(Mutex::new(Config {
            paths: vec![
                WatchedFolder {
                    id: 1,
                    path: "foo".into(),
                },
                WatchedFolder {
                    id: 2,
                    path: "bar".into(),
                },
            ],
            peers: vec![],
        }));
        let (tx, _) = mpsc::channel(1);

        let app = app(config.clone(), tx, false);

        let folder_to_update = WatchedFolder {
            id: 3,
            path: "foobar".into(),
        };

        let json_body = to_string(&folder_to_update).unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/folder")
                    .header("Content-Type", "application/json")
                    .body(Body::from(json_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);

        assert_eq!(config.lock().unwrap().paths.len(), 3);
        assert_eq!(config.lock().unwrap().paths[2].id, 3);
        assert_eq!(
            config.lock().unwrap().paths[2].path.to_string_lossy(),
            "foobar"
        );
    }
}
