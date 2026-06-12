//! GeneGIS Server — collab session + agent run sync prototype (Phase 5–7 alpha).

use std::net::SocketAddr;
use std::sync::Arc;

use genegis_server::agent_store::{
    AgentRunStore, DEFAULT_AGENT_LATEST_PATH, DEFAULT_AGENT_RUNS_DIR,
};
use genegis_server::api::{serve, AppState};
use genegis_server::store::{CollabStore, DEFAULT_COLLAB_PATH};

#[tokio::main]
async fn main() {
    let collab_path = std::env::var("GENEGIS_COLLAB_PATH")
        .unwrap_or_else(|_| DEFAULT_COLLAB_PATH.into());
    let agent_runs_dir = std::env::var("GENEGIS_AGENT_RUNS_DIR")
        .unwrap_or_else(|_| DEFAULT_AGENT_RUNS_DIR.into());
    let agent_latest_path = std::env::var("GENEGIS_AGENT_LATEST_PATH")
        .unwrap_or_else(|_| DEFAULT_AGENT_LATEST_PATH.into());

    let collab = Arc::new(CollabStore::load(&collab_path));
    let agent_runs = Arc::new(AgentRunStore::load(&agent_runs_dir, &agent_latest_path));
    let state = AppState {
        collab: Arc::clone(&collab),
        agent_runs: Arc::clone(&agent_runs),
    };

    let port: u16 = std::env::var("GENEGIS_SERVER_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(7813);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("GeneGIS Server listening on http://{addr}/");
    println!("Collab JSON: {}", collab.json_path().display());
    println!("Collab Automerge: {}", collab.snapshot_path().display());
    println!("Agent runs: {}", agent_runs.runs_dir().display());
    println!("Agent latest: {}", agent_runs.latest_path().display());

    serve(state, addr).await.expect("serve");
}
