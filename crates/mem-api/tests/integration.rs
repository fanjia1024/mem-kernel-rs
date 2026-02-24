//! Integration tests: add/search, update, forget, get_memory, isolation.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use mem_api::server::{self, AppState, InMemoryAuditStore};
use mem_cube::NaiveMemCube;
use mem_embed::MockEmbedder;
use mem_graph::InMemoryGraphStore;
use mem_scheduler::InMemoryScheduler;
use mem_vec::InMemoryVecStore;
use serde_json::json;
use std::sync::Arc;
use tower::util::ServiceExt;

fn test_app() -> axum::Router {
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
    let id = j["data"].as_array().and_then(|a| a.first()).and_then(|d| d.get("id")).and_then(|v| v.as_str()).unwrap();

    let search_body = json!({ "query": "What do I like?", "user_id": "user1", "mem_cube_id": "user1" });
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
    let task_id = j["data"].as_array().and_then(|a| a.first()).and_then(|d| d.get("task_id")).and_then(|v| v.as_str()).unwrap();

    for _ in 0..50 {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/product/scheduler/status?user_id=u2&task_id={}", task_id))
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
        .uri(format!("/product/scheduler/status?user_id=u2&task_id={}", task_id))
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
    let id = j["data"].as_array().and_then(|a| a.first()).and_then(|d| d.get("id")).and_then(|v| v.as_str()).unwrap().to_string();

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
    let id = j["data"].as_array().and_then(|a| a.first()).and_then(|d| d.get("id")).and_then(|v| v.as_str()).unwrap().to_string();

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
    let id = j["data"].as_array().and_then(|a| a.first()).and_then(|d| d.get("id")).and_then(|v| v.as_str()).unwrap().to_string();

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
    let id_a = j["data"].as_array().and_then(|a| a.first()).and_then(|d| d.get("id")).and_then(|v| v.as_str()).unwrap().to_string();

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
