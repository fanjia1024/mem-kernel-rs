//! Integration tests: add/search, update, forget, get_memory, isolation.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use mem_api::server::{self, AppState, InMemoryAuditStore};
use mem_cube::NaiveMemCube;
use mem_embed::MockEmbedder;
use mem_graph::InMemoryGraphStore;
use mem_scheduler::InMemoryScheduler;
use mem_vec::InMemoryVecStore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tower::util::ServiceExt;

fn test_app() -> axum::Router {
    test_app_with_auth(None)
}

fn test_app_with_auth(auth_token: Option<&str>) -> axum::Router {
    let graph = InMemoryGraphStore::new();
    let vec_store = InMemoryVecStore::new(None);
    let embedder = MockEmbedder::new();
    let cube: Arc<dyn mem_types::MemCube + Send + Sync> =
        Arc::new(NaiveMemCube::new(graph, vec_store, embedder));
    let audit_store: Arc<dyn mem_types::AuditStore + Send + Sync> =
        Arc::new(InMemoryAuditStore::new());
    let scheduler = Arc::new(InMemoryScheduler::new(
        Arc::clone(&cube),
        Some(Arc::clone(&audit_store)),
    ));
    let state = Arc::new(AppState {
        cube,
        scheduler,
        audit_log: audit_store,
        auth_token: auth_token.map(str::to_string),
    });
    server::router(state)
}

#[tokio::test]
async fn add_sync_then_search() {
    let app = test_app();
    let add_body = json!({
        "user_id": "user1",
        "mem_cube_id": "user1",
        "memory_content": "I like strawberries",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 200);
    let id = j["data"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .unwrap();

    let search_body =
        json!({ "query": "What do I like?", "user_id": "user1", "mem_cube_id": "user1" });
    let req = Request::builder()
        .method("POST")
        .uri("/product/search")
        .header("content-type", "application/json")
        .body(Body::from(search_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let memories = j["data"]["text_mem"][0]["memories"].as_array().unwrap();
    assert!(!memories.is_empty());
    assert_eq!(memories[0]["memory"], "I like strawberries");
    assert_eq!(memories[0]["id"], id);
}

#[tokio::test]
async fn add_async_then_status_then_search() {
    let app = test_app();
    let add_body = json!({
        "user_id": "u2",
        "memory_content": "Async memory content",
        "async_mode": "async"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_id = j["data"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|d| d.get("task_id"))
        .and_then(|v| v.as_str())
        .unwrap();

    for _ in 0..50 {
        let req = Request::builder()
            .method("GET")
            .uri(format!(
                "/product/scheduler/status?user_id=u2&task_id={}",
                task_id
            ))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let status = j["data"]["status"].as_str().unwrap_or("");
        if status == "done" {
            break;
        }
        if status == "failed" {
            panic!("async job failed: {:?}", j["data"]["result_summary"]);
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/product/scheduler/status?user_id=u2&task_id={}",
            task_id
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 200);
    assert_eq!(j["data"]["status"], "done");

    let search_body = json!({ "query": "Async", "user_id": "u2" });
    let req = Request::builder()
        .method("POST")
        .uri("/product/search")
        .header("content-type", "application/json")
        .body(Body::from(search_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let memories = j["data"]["text_mem"][0]["memories"].as_array().unwrap();
    assert!(!memories.is_empty());
    assert_eq!(memories[0]["memory"], "Async memory content");
}

#[tokio::test]
async fn update_memory_then_search() {
    let app = test_app();
    let add_body = json!({
        "user_id": "u3",
        "memory_content": "Original text",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id = j["data"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    let update_body = json!({
        "memory_id": id,
        "user_id": "u3",
        "memory": "Updated text"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/update_memory")
        .header("content-type", "application/json")
        .body(Body::from(update_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let search_body = json!({ "query": "Updated", "user_id": "u3" });
    let req = Request::builder()
        .method("POST")
        .uri("/product/search")
        .header("content-type", "application/json")
        .body(Body::from(search_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let memories = j["data"]["text_mem"][0]["memories"].as_array().unwrap();
    assert!(!memories.is_empty());
    assert_eq!(memories[0]["memory"], "Updated text");
}

#[tokio::test]
async fn forget_soft_then_search_misses_get_with_include_deleted() {
    let app = test_app();
    let add_body = json!({
        "user_id": "u4",
        "memory_content": "To be soft deleted",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id = j["data"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    let del_body = json!({ "memory_id": id, "user_id": "u4", "soft": true });
    let req = Request::builder()
        .method("POST")
        .uri("/product/delete_memory")
        .header("content-type", "application/json")
        .body(Body::from(del_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let search_body = json!({ "query": "soft deleted", "user_id": "u4" });
    let req = Request::builder()
        .method("POST")
        .uri("/product/search")
        .header("content-type", "application/json")
        .body(Body::from(search_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let memories = j["data"]["text_mem"][0]["memories"].as_array().unwrap();
    assert!(memories.is_empty());

    let get_body = json!({ "memory_id": id, "user_id": "u4" });
    let req = Request::builder()
        .method("POST")
        .uri("/product/get_memory")
        .header("content-type", "application/json")
        .body(Body::from(get_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 404);

    let get_body = json!({ "memory_id": id, "user_id": "u4", "include_deleted": true });
    let req = Request::builder()
        .method("POST")
        .uri("/product/get_memory")
        .header("content-type", "application/json")
        .body(Body::from(get_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 200);
    assert_eq!(j["data"]["metadata"]["state"], "tombstone");
}

#[tokio::test]
async fn forget_hard_then_get_404() {
    let app = test_app();
    let add_body = json!({
        "user_id": "u5",
        "memory_content": "To be hard deleted",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id = j["data"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    let del_body = json!({ "memory_id": id, "user_id": "u5", "soft": false });
    let req = Request::builder()
        .method("POST")
        .uri("/product/delete_memory")
        .header("content-type", "application/json")
        .body(Body::from(del_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let get_body = json!({ "memory_id": id, "user_id": "u5" });
    let req = Request::builder()
        .method("POST")
        .uri("/product/get_memory")
        .header("content-type", "application/json")
        .body(Body::from(get_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 404);
}

#[tokio::test]
async fn update_delete_nonexistent_404() {
    let app = test_app();
    let update_body = json!({
        "memory_id": "nonexistent-id",
        "user_id": "u6"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/update_memory")
        .header("content-type", "application/json")
        .body(Body::from(update_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 404);

    let del_body = json!({ "memory_id": "nonexistent-id", "user_id": "u6" });
    let req = Request::builder()
        .method("POST")
        .uri("/product/delete_memory")
        .header("content-type", "application/json")
        .body(Body::from(del_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 404);
}

#[tokio::test]
async fn multi_user_isolation() {
    let app = test_app();
    let add_a = json!({ "user_id": "alice", "mem_cube_id": "alice", "memory_content": "Alice secret", "async_mode": "sync" });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_a.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id_a = j["data"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    let add_b = json!({ "user_id": "bob", "mem_cube_id": "bob", "memory_content": "Bob secret", "async_mode": "sync" });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_b.to_string()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    let search_bob = json!({ "query": "secret", "user_id": "bob" });
    let req = Request::builder()
        .method("POST")
        .uri("/product/search")
        .header("content-type", "application/json")
        .body(Body::from(search_bob.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let memories = j["data"]["text_mem"][0]["memories"].as_array().unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0]["memory"], "Bob secret");

    let get_as_bob = json!({ "memory_id": id_a, "user_id": "bob" });
    let req = Request::builder()
        .method("POST")
        .uri("/product/get_memory")
        .header("content-type", "application/json")
        .body(Body::from(get_as_bob.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 404);
}

#[tokio::test]
async fn scheduler_status_requires_owner_user() {
    let app = test_app();
    let add_body = json!({
        "user_id": "owner",
        "memory_content": "owner async item",
        "async_mode": "async"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_id = j["data"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|d| d.get("task_id"))
        .and_then(|v| v.as_str())
        .unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/product/scheduler/status?user_id=intruder&task_id={}",
            task_id
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 404);
}

#[tokio::test]
async fn search_filter_cannot_break_cube_isolation() {
    let app = test_app();

    let add_alice = json!({
        "user_id": "alice2",
        "mem_cube_id": "alice2",
        "memory_content": "Alice private",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_alice.to_string()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    let add_bob = json!({
        "user_id": "bob2",
        "mem_cube_id": "bob2",
        "memory_content": "Bob private",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_bob.to_string()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    let search_body = json!({
        "query": "private",
        "user_id": "bob2",
        "filter": { "mem_cube_id": "alice2" }
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/search")
        .header("content-type", "application/json")
        .body(Body::from(search_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let memories = j["data"]["text_mem"][0]["memories"].as_array().unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0]["memory"], "Bob private");
}

#[tokio::test]
async fn search_respects_relativity_threshold() {
    let app = test_app();
    let add_body = json!({
        "user_id": "u_rel",
        "mem_cube_id": "u_rel",
        "memory_content": "Relativity target memory",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    let baseline_search = json!({
        "query": "Relativity target memory",
        "user_id": "u_rel",
        "top_k": 1
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/search")
        .header("content-type", "application/json")
        .body(Body::from(baseline_search.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let score = j["data"]["text_mem"][0]["memories"][0]["metadata"]["relativity"]
        .as_f64()
        .unwrap();

    let strict_search = json!({
        "query": "Relativity target memory",
        "user_id": "u_rel",
        "top_k": 1,
        "relativity": score + 0.000001
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/search")
        .header("content-type", "application/json")
        .body(Body::from(strict_search.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let memories = j["data"]["text_mem"][0]["memories"].as_array().unwrap();
    assert!(memories.is_empty());
}

#[tokio::test]
async fn auth_rejects_request_without_bearer_token() {
    let app = test_app_with_auth(Some("secret-token"));
    let add_body = json!({
        "user_id": "auth-u1",
        "memory_content": "auth required",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 401);
}

#[tokio::test]
async fn auth_accepts_request_with_valid_bearer_token() {
    let app = test_app_with_auth(Some("secret-token"));
    let add_body = json!({
        "user_id": "auth-u2",
        "memory_content": "auth pass",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .header(header::AUTHORIZATION, "Bearer secret-token")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 200);
}

#[tokio::test]
async fn metrics_endpoint_exposes_prometheus_text() {
    let app = test_app();
    let req = Request::builder()
        .method("GET")
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("mem_api_requests_total"));
    assert!(text.contains("mem_api_errors_total"));
    assert!(text.contains("mem_api_request_duration_ms"));
}

#[tokio::test]
async fn request_id_is_echoed_and_written_to_audit() {
    let app = test_app();
    let add_body = json!({
        "user_id": "rid_u1",
        "mem_cube_id": "rid_u1",
        "memory_content": "request id tracing",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .header("x-request-id", "req-12345")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let header_val = res
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(header_val, "req-12345");

    let req = Request::builder()
        .method("GET")
        .uri("/product/audit/list?user_id=rid_u1&cube_id=rid_u1&limit=1")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 200);
    let summary = j["data"][0]["input_summary"].as_str().unwrap_or("");
    assert!(summary.contains("request_id=req-12345"));
}

#[tokio::test]
async fn add_with_relations_then_query_neighbors() {
    let app = test_app();

    let first = json!({
        "user_id": "g1",
        "mem_cube_id": "g1",
        "memory_content": "Graph base node",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(first.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id1 = j["data"][0]["id"].as_str().unwrap().to_string();

    let second = json!({
        "user_id": "g1",
        "mem_cube_id": "g1",
        "memory_content": "Graph linked node",
        "async_mode": "sync",
        "relations": [
            {
                "memory_id": id1,
                "relation": "related_to",
                "direction": "outbound"
            }
        ]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(second.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id2 = j["data"][0]["id"].as_str().unwrap().to_string();

    let neighbors_body = json!({
        "memory_id": id2,
        "user_id": "g1",
        "direction": "outbound",
        "relation": "related_to",
        "limit": 10
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/graph/neighbors")
        .header("content-type", "application/json")
        .body(Body::from(neighbors_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 200);
    let data = j["data"]["items"].as_array().unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["edge"]["relation"], "related_to");
    assert_eq!(data[0]["memory"]["id"], id1);
    assert_eq!(data[0]["memory"]["memory"], "Graph base node");
}

#[tokio::test]
async fn graph_neighbors_supports_cursor_pagination() {
    let app = test_app();

    let root = json!({
        "user_id": "g_page",
        "mem_cube_id": "g_page",
        "memory_content": "Root",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(root.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let root_id = j["data"][0]["id"].as_str().unwrap().to_string();

    for i in 0..3 {
        let add = json!({
            "user_id": "g_page",
            "mem_cube_id": "g_page",
            "memory_content": format!("Leaf {}", i),
            "async_mode": "sync",
            "relations": [{
                "memory_id": root_id,
                "relation": "related_to",
                "direction": "outbound"
            }]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/product/add")
            .header("content-type", "application/json")
            .body(Body::from(add.to_string()))
            .unwrap();
        let _ = app.clone().oneshot(req).await.unwrap();
    }

    let page1 = json!({
        "memory_id": root_id,
        "user_id": "g_page",
        "direction": "inbound",
        "relation": "related_to",
        "limit": 2
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/graph/neighbors")
        .header("content-type", "application/json")
        .body(Body::from(page1.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j1: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items1 = j1["data"]["items"].as_array().unwrap();
    assert_eq!(items1.len(), 2);
    let cursor = j1["data"]["next_cursor"].as_str().unwrap().to_string();

    let page2 = json!({
        "memory_id": root_id,
        "user_id": "g_page",
        "direction": "inbound",
        "relation": "related_to",
        "limit": 2,
        "cursor": cursor
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/graph/neighbors")
        .header("content-type", "application/json")
        .body(Body::from(page2.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j2: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items2 = j2["data"]["items"].as_array().unwrap();
    assert_eq!(items2.len(), 1);
    assert!(j2["data"]["next_cursor"].is_null());
}

#[tokio::test]
async fn graph_neighbors_invalid_cursor_returns_400() {
    let app = test_app();
    let req_body = json!({
        "memory_id": "does-not-matter",
        "user_id": "u1",
        "cursor": "not-a-number"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/graph/neighbors")
        .header("content-type", "application/json")
        .body(Body::from(req_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 400);
}

#[tokio::test]
async fn relation_cannot_link_to_other_tenant_and_rolls_back() {
    let app = test_app();

    let add_alice = json!({
        "user_id": "alice_g",
        "mem_cube_id": "alice_g",
        "memory_content": "Alice graph root",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_alice.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let alice_id = j["data"][0]["id"].as_str().unwrap().to_string();

    let add_bob = json!({
        "user_id": "bob_g",
        "mem_cube_id": "bob_g",
        "memory_content": "Bob should rollback",
        "async_mode": "sync",
        "relations": [
            {
                "memory_id": alice_id,
                "relation": "related_to",
                "direction": "outbound"
            }
        ]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_bob.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 500);

    // Bob add should be rolled back entirely.
    let search_bob = json!({
        "query": "rollback",
        "user_id": "bob_g"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/search")
        .header("content-type", "application/json")
        .body(Body::from(search_bob.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let memories = j["data"]["text_mem"][0]["memories"].as_array().unwrap();
    assert!(memories.is_empty());
}

#[tokio::test]
async fn graph_edges_are_removed_when_node_deleted() {
    let app = test_app();

    let first = json!({
        "user_id": "g_del",
        "mem_cube_id": "g_del",
        "memory_content": "Delete graph base",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(first.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id1 = j["data"][0]["id"].as_str().unwrap().to_string();

    let second = json!({
        "user_id": "g_del",
        "mem_cube_id": "g_del",
        "memory_content": "Delete graph linked",
        "async_mode": "sync",
        "relations": [
            {
                "memory_id": id1,
                "relation": "depends_on",
                "direction": "outbound"
            }
        ]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(second.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id2 = j["data"][0]["id"].as_str().unwrap().to_string();

    let neighbors_before = json!({
        "memory_id": id1,
        "user_id": "g_del",
        "direction": "inbound",
        "relation": "depends_on",
        "limit": 10
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/graph/neighbors")
        .header("content-type", "application/json")
        .body(Body::from(neighbors_before.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let before = j["data"]["items"].as_array().unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0]["memory"]["id"], id2);

    let del_body = json!({ "memory_id": id2, "user_id": "g_del", "soft": false });
    let req = Request::builder()
        .method("POST")
        .uri("/product/delete_memory")
        .header("content-type", "application/json")
        .body(Body::from(del_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let neighbors_after = json!({
        "memory_id": id1,
        "user_id": "g_del",
        "direction": "inbound",
        "relation": "depends_on",
        "limit": 10
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/graph/neighbors")
        .header("content-type", "application/json")
        .body(Body::from(neighbors_after.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let after = j["data"]["items"].as_array().unwrap();
    assert!(after.is_empty());
}

#[tokio::test]
async fn graph_path_returns_shortest_hops() {
    let app = test_app();

    let add_a = json!({
        "user_id": "g_path",
        "mem_cube_id": "g_path",
        "memory_content": "Path A",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_a.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id_a = j["data"][0]["id"].as_str().unwrap().to_string();

    let add_b = json!({
        "user_id": "g_path",
        "mem_cube_id": "g_path",
        "memory_content": "Path B",
        "async_mode": "sync",
        "relations": [{
            "memory_id": id_a,
            "relation": "depends_on",
            "direction": "outbound"
        }]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_b.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id_b = j["data"][0]["id"].as_str().unwrap().to_string();

    let add_c = json!({
        "user_id": "g_path",
        "mem_cube_id": "g_path",
        "memory_content": "Path C",
        "async_mode": "sync",
        "relations": [{
            "memory_id": id_b,
            "relation": "depends_on",
            "direction": "outbound"
        }]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_c.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id_c = j["data"][0]["id"].as_str().unwrap().to_string();

    let path_req = json!({
        "source_memory_id": id_c,
        "target_memory_id": id_a,
        "user_id": "g_path",
        "direction": "outbound",
        "relation": "depends_on",
        "max_depth": 4
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/graph/path")
        .header("content-type", "application/json")
        .body(Body::from(path_req.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 200);
    assert_eq!(j["data"]["hops"], 2);
    let nodes = j["data"]["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3);
    assert_eq!(nodes[0]["id"], id_c);
    assert_eq!(nodes[1]["id"], id_b);
    assert_eq!(nodes[2]["id"], id_a);
    let edges = j["data"]["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
}

#[tokio::test]
async fn graph_paths_returns_multiple_candidates() {
    let app = test_app();

    let add_s = json!({
        "user_id": "g_paths",
        "mem_cube_id": "g_paths",
        "memory_content": "S",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_s.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let s_id = j["data"][0]["id"].as_str().unwrap().to_string();

    let add_a = json!({
        "user_id": "g_paths",
        "mem_cube_id": "g_paths",
        "memory_content": "A",
        "async_mode": "sync",
        "relations": [{
            "memory_id": s_id,
            "relation": "r",
            "direction": "inbound"
        }]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_a.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let a_id = j["data"][0]["id"].as_str().unwrap().to_string();

    let add_b = json!({
        "user_id": "g_paths",
        "mem_cube_id": "g_paths",
        "memory_content": "B",
        "async_mode": "sync",
        "relations": [{
            "memory_id": s_id,
            "relation": "r",
            "direction": "inbound"
        }]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_b.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let b_id = j["data"][0]["id"].as_str().unwrap().to_string();

    let add_t = json!({
        "user_id": "g_paths",
        "mem_cube_id": "g_paths",
        "memory_content": "T",
        "async_mode": "sync",
        "relations": [
            { "memory_id": a_id, "relation": "r", "direction": "inbound" },
            { "memory_id": b_id, "relation": "r", "direction": "inbound" }
        ]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_t.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let t_id = j["data"][0]["id"].as_str().unwrap().to_string();

    let req_body = json!({
        "source_memory_id": s_id,
        "target_memory_id": t_id,
        "user_id": "g_paths",
        "direction": "outbound",
        "relation": "r",
        "max_depth": 4,
        "top_k_paths": 2
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/graph/paths")
        .header("content-type", "application/json")
        .body(Body::from(req_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 200);
    let paths = j["data"].as_array().unwrap();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0]["hops"], 2);
    assert_eq!(paths[1]["hops"], 2);
}

#[tokio::test]
async fn graph_paths_invalid_top_k_returns_400() {
    let app = test_app();
    let req_body = json!({
        "source_memory_id": "a",
        "target_memory_id": "b",
        "user_id": "u1",
        "top_k_paths": 0
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/graph/paths")
        .header("content-type", "application/json")
        .body(Body::from(req_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 400);
}

#[derive(Debug, Deserialize, Serialize)]
struct ComplexMemoryFixture {
    user_id: String,
    mem_cube_id: String,
    profile: HashMap<String, serde_json::Value>,
    recent_chat: Vec<FixtureMessage>,
    memories: Vec<FixtureMemory>,
    questions: Vec<FixtureQuestion>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FixtureMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct FixtureMemory {
    stage: String,
    scope: String,
    memory_content: String,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FixtureQuestion {
    stage: String,
    scope: String,
    query: String,
    expected_contains: String,
}

#[derive(Debug, Deserialize)]
struct CsvDatasetRow {
    row_type: String,
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    mem_cube_id: String,
    #[serde(default)]
    stage: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    query: String,
    #[serde(default)]
    expected_contains: String,
    #[serde(default)]
    tags: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    city: String,
    #[serde(default)]
    job: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    timezone: String,
}

fn split_tags(raw: &str) -> Vec<String> {
    raw.split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn load_complex_memory_fixture() -> ComplexMemoryFixture {
    let path = format!(
        "{}/tests/fixtures/complex_dialogue_memory.csv",
        env!("CARGO_MANIFEST_DIR")
    );

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)
        .unwrap();

    let mut user_id = String::new();
    let mut mem_cube_id = String::new();
    let mut profile = HashMap::new();
    let mut recent_chat = Vec::new();
    let mut memories = Vec::new();
    let mut questions = Vec::new();

    for row in rdr.deserialize::<CsvDatasetRow>() {
        let row = row.unwrap();
        match row.row_type.as_str() {
            "profile" => {
                user_id = row.user_id;
                mem_cube_id = row.mem_cube_id;
                profile.insert("name".to_string(), serde_json::Value::String(row.name));
                profile.insert("city".to_string(), serde_json::Value::String(row.city));
                profile.insert("job".to_string(), serde_json::Value::String(row.job));
                profile.insert(
                    "language".to_string(),
                    serde_json::Value::String(row.language),
                );
                profile.insert(
                    "timezone".to_string(),
                    serde_json::Value::String(row.timezone),
                );
            }
            "chat" => {
                recent_chat.push(FixtureMessage {
                    role: row.role,
                    content: row.content,
                });
            }
            "memory" => {
                memories.push(FixtureMemory {
                    stage: row.stage,
                    scope: row.scope,
                    memory_content: row.content,
                    tags: split_tags(&row.tags),
                });
            }
            "question" => {
                questions.push(FixtureQuestion {
                    stage: row.stage,
                    scope: row.scope,
                    query: row.query,
                    expected_contains: row.expected_contains,
                });
            }
            other => panic!("unsupported row_type: {}", other),
        }
    }

    assert!(!user_id.is_empty(), "missing profile row user_id");
    assert!(!mem_cube_id.is_empty(), "missing profile row mem_cube_id");
    assert!(!recent_chat.is_empty(), "missing chat rows");
    assert!(!memories.is_empty(), "missing memory rows");
    assert!(!questions.is_empty(), "missing question rows");

    ComplexMemoryFixture {
        user_id,
        mem_cube_id,
        profile,
        recent_chat,
        memories,
        questions,
    }
}

#[tokio::test]
async fn complex_dialogue_memory_returns_short_mid_long_term() {
    let app = test_app();
    let fixture = load_complex_memory_fixture();
    for (stage, scope) in [
        ("short_term", "WorkingMemory"),
        ("mid_term", "UserMemory"),
        ("long_term", "LongTermMemory"),
    ] {
        assert!(
            fixture
                .memories
                .iter()
                .any(|m| m.stage == stage && m.scope == scope),
            "dataset missing memory stage={} scope={}",
            stage,
            scope
        );
        assert!(
            fixture
                .questions
                .iter()
                .any(|q| q.stage == stage && q.scope == scope),
            "dataset missing question stage={} scope={}",
            stage,
            scope
        );
    }

    for item in &fixture.memories {
        let mut info = fixture.profile.clone();
        info.insert(
            "scope".to_string(),
            serde_json::Value::String(item.scope.clone()),
        );
        info.insert(
            "memory_stage".to_string(),
            serde_json::Value::String(item.stage.clone()),
        );

        let add_body = json!({
            "user_id": &fixture.user_id,
            "mem_cube_id": &fixture.mem_cube_id,
            "session_id": "sess-2026-02-25-complex",
            "memory_content": &item.memory_content,
            "chat_history": &fixture.recent_chat,
            "custom_tags": &item.tags,
            "info": info,
            "async_mode": "sync"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/product/add")
            .header("content-type", "application/json")
            .body(Body::from(add_body.to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(j["code"], 200);
    }

    let req = Request::builder()
        .method("POST")
        .uri("/product/search")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "query": "Q: 明天长期目标是什么？A: 在 2026 年 Q3 前通过 Rust 异步系统设计面试，并每周六完成一次系统设计复盘。",
                "user_id": &fixture.user_id,
                "mem_cube_id": &fixture.mem_cube_id,
                "top_k": 10
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let buckets = j["data"]["text_mem"].as_array().unwrap();
    assert!(buckets.iter().any(|b| b["name"] == "short_term"));
    assert!(buckets.iter().any(|b| b["name"] == "mid_term"));
    assert!(buckets.iter().any(|b| b["name"] == "long_term"));

    for q in &fixture.questions {
        let search_body = json!({
            "query": &q.query,
            "user_id": &fixture.user_id,
            "mem_cube_id": &fixture.mem_cube_id,
            "top_k": 5,
            "filter": {
                "scope": &q.scope
            }
        });
        let req = Request::builder()
            .method("POST")
            .uri("/product/search")
            .header("content-type", "application/json")
            .body(Body::from(search_body.to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(j["code"], 200);
        let memories = j["data"]["text_mem"][0]["memories"].as_array().unwrap();
        assert!(
            !memories.is_empty(),
            "no memory found for stage={} scope={}",
            q.stage,
            q.scope
        );
        assert!(memories.iter().all(|m| m["metadata"]["scope"] == q.scope));
        assert!(memories.iter().any(|m| {
            m["memory"]
                .as_str()
                .map(|text| text.contains(&q.expected_contains))
                .unwrap_or(false)
        }));
    }
}

#[tokio::test]
async fn hybrid_search_vector_only_returns_fused_hits() {
    let app = test_app();
    let add_body = json!({
        "user_id": "hybrid_user",
        "mem_cube_id": "hybrid_user",
        "memory_content": "Rust is a systems programming language focused on safety.",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 200);
    let memory_id = j["data"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .unwrap();

    let hybrid_body = json!({
        "user_id": "hybrid_user",
        "mem_cube_id": "hybrid_user",
        "query": "systems programming language",
        "top_k": 5,
        "mode": "vector_only"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/hybrid_search")
        .header("content-type", "application/json")
        .body(Body::from(hybrid_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 200);
    let data = j["data"].as_object().expect("data present");
    assert!(data.contains_key("latency_ms"));
    let hits = data["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "expected at least one hit");
    assert_eq!(hits[0]["memory_id"], memory_id);
    assert!(hits[0].get("fused_score").is_some());
}

#[tokio::test]
async fn hybrid_search_fusion_returns_channel_results() {
    let app = test_app();
    let add_body = json!({
        "user_id": "fusion_user",
        "mem_cube_id": "fusion_user",
        "memory_content": "Meeting with the team tomorrow at 3pm.",
        "async_mode": "sync"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/add")
        .header("content-type", "application/json")
        .body(Body::from(add_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let hybrid_body = json!({
        "user_id": "fusion_user",
        "query": "team meeting",
        "top_k": 5,
        "mode": "fusion"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/product/hybrid_search")
        .header("content-type", "application/json")
        .body(Body::from(hybrid_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(j["code"], 200);
    let data = j["data"].as_object().expect("data present");
    assert!(data.contains_key("latency_ms"));
    assert!(data.contains_key("channel_results"));
    let hits = data["hits"].as_array().expect("hits array");
    if !hits.is_empty() {
        assert!(hits[0].get("fused_score").is_some());
    }
}
