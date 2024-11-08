use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use std::sync::{Arc, Mutex};

use crate::config::Config;

type MutexConf = Arc<Mutex<Config>>;

pub fn app(config: MutexConf) -> Router {
    Router::new()
        .route("/", get(config_index))
        .with_state(config)
}

async fn config_index(State(conf): State<MutexConf>) -> Json<Config> {
    let config = conf.lock().unwrap();
    Json(config.clone())
}
