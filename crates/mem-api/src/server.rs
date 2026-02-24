//! Axum server and routes.

use axum::{extract::State, routing::post, Json, Router};
use mem_types::{ApiAddRequest, ApiSearchRequest, MemoryResponse, SearchResponse};
use mem_types::MemCube;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

pub type AppState = Arc<dyn MemCube + Send + Sync>;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/product/add", post(handle_add))
        .route("/product/search", post(handle_search))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn handle_add(
    State(cube): State<AppState>,
    Json(req): Json<ApiAddRequest>,
) -> Json<MemoryResponse> {
    match cube.add_memories(&req).await {
        Ok(res) => Json(res),
        Err(e) => Json(MemoryResponse {
            code: 500,
            message: e.to_string(),
            data: None,
        }),
    }
}

async fn handle_search(
    State(cube): State<AppState>,
    Json(req): Json<ApiSearchRequest>,
) -> Json<SearchResponse> {
    match cube.search_memories(&req).await {
        Ok(res) => Json(res),
        Err(e) => Json(SearchResponse {
            code: 500,
            message: e.to_string(),
            data: None,
        }),
    }
}
