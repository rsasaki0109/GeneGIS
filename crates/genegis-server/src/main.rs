//! GeneGIS Server — collab session sync prototype (Phase 5 alpha).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use genegis_collab::CollabSession;
use genegis_server::store::{CollabStore, DEFAULT_COLLAB_PATH};
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct AppState {
    store: Arc<CollabStore>,
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
    collab_path: String,
}

#[derive(Serialize)]
struct CollabResponse {
    ok: bool,
    summary: serde_json::Value,
    comments: serde_json::Value,
    session: Option<String>,
}

#[derive(Deserialize)]
struct CollabUpload {
    session: String,
}

#[tokio::main]
async fn main() {
    let collab_path = std::env::var("GENEGIS_COLLAB_PATH")
        .unwrap_or_else(|_| DEFAULT_COLLAB_PATH.into());
    let store = Arc::new(CollabStore::load(&collab_path));
    let state = AppState {
        store: Arc::clone(&store),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/collab", get(get_collab).put(put_collab))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let port: u16 = std::env::var("GENEGIS_SERVER_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(7813);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("GeneGIS Server listening on http://{addr}/");
    println!("Collab store: {}", store.path().display());

    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "genegis-server",
        collab_path: state.store.path().display().to_string(),
    })
}

async fn get_collab(State(state): State<AppState>) -> impl IntoResponse {
    match state.store.snapshot() {
        Ok(session) => collab_ok(session, true),
        Err(err) => collab_error(err.to_string()),
    }
}

async fn put_collab(
    State(state): State<AppState>,
    Json(body): Json<CollabUpload>,
) -> impl IntoResponse {
    match state.store.replace_json(&body.session) {
        Ok(session) => collab_ok(session, false),
        Err(err) => collab_error(err.to_string()),
    }
}

fn collab_ok(session: CollabSession, include_session: bool) -> (StatusCode, Json<CollabResponse>) {
    (
        StatusCode::OK,
        Json(CollabResponse {
            ok: true,
            summary: session.summary_json(),
            comments: session.comments_json(),
            session: if include_session {
                session.export_json().ok()
            } else {
                None
            },
        }),
    )
}

fn collab_error(message: String) -> (StatusCode, Json<CollabResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(CollabResponse {
            ok: false,
            summary: serde_json::json!({ "error": message }),
            comments: serde_json::json!([]),
            session: None,
        }),
    )
}
