//! Phase 7 gamma — server agent run API smoke (in-process axum).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use genegis_agent::{AgentOrchestrator, AgentRunConfig};
use genegis_server::agent_store::AgentRunStore;
use genegis_server::api::{build_router, AppState};
use genegis_server::store::CollabStore;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

fn test_state(temp: &tempfile::TempDir) -> AppState {
    let collab_path = temp.path().join("collab.json");
    let runs_dir = temp.path().join("agent-runs");
    let latest_path = temp.path().join("agent-run.json");
    AppState {
        collab: Arc::new(CollabStore::load(&collab_path)),
        agent_runs: Arc::new(AgentRunStore::load(&runs_dir, &latest_path)),
    }
}

async fn read_json_body(body: Body) -> Value {
    let bytes = body.collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

fn expect_ok_run(json: &Value) -> uuid::Uuid {
    assert_eq!(json.get("ok").and_then(Value::as_bool), Some(true));
    let id = json
        .pointer("/run/id")
        .and_then(Value::as_str)
        .expect("run id");
    uuid::Uuid::parse_str(id).expect("uuid")
}

#[tokio::test]
async fn agent_run_api_round_trip_north_star() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_router(test_state(&temp));

    let run = AgentOrchestrator::new()
        .with_config(AgentRunConfig::rule_based_offline())
        .run("名古屋市の人口密度を表示")
        .expect("north star run");
    assert!(run.verification_passed);

    let post_body = serde_json::to_string(&run).expect("json");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/runs")
                .header("content-type", "application/json")
                .body(Body::from(post_body))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let posted = read_json_body(response.into_body()).await;
    assert_eq!(expect_ok_run(&posted), run.id);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agent/runs")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let listed = read_json_body(response.into_body()).await;
    assert_eq!(listed.get("ok").and_then(Value::as_bool), Some(true));
    let runs = listed
        .get("runs")
        .and_then(Value::as_array)
        .expect("runs array");
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].get("id").and_then(Value::as_str),
        Some(run.id.to_string().as_str())
    );
    assert_eq!(
        runs[0]
            .get("verification_passed")
            .and_then(Value::as_bool),
        Some(true)
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent/runs/{}", run.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let fetched = read_json_body(response.into_body()).await;
    assert_eq!(
        fetched.pointer("/run/steps").and_then(Value::as_array).map(Vec::len),
        Some(4)
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/agent/runs/latest")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let latest = read_json_body(response.into_body()).await;
    assert_eq!(expect_ok_run(&latest), run.id);
}

#[tokio::test]
async fn agent_run_api_records_plan_only_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_router(test_state(&temp));

    let run = AgentOrchestrator::new()
        .with_config(AgentRunConfig::rule_based_offline().plan_only())
        .run("名古屋市の人口密度を表示")
        .expect("plan-only run");

    let post_body = serde_json::to_string(&run).expect("json");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/runs")
                .header("content-type", "application/json")
                .body(Body::from(post_body))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/agent/runs")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let listed = read_json_body(response.into_body()).await;
    let runs = listed
        .get("runs")
        .and_then(Value::as_array)
        .expect("runs array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].get("plan_only").and_then(Value::as_bool), Some(true));
    assert_eq!(
        runs[0]
            .get("verification_passed")
            .and_then(Value::as_bool),
        Some(false)
    );
}
