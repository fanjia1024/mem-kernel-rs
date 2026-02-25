//! Axum server and routes.

use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use mem_scheduler::Scheduler;
use mem_types::MemCube;
use mem_types::{
    ApiAddRequest, ApiSearchRequest, AuditEvent, AuditEventKind, AuditListOptions, AuditStore,
    ForgetMemoryRequest, ForgetMemoryResponse, GetMemoryRequest, GetMemoryResponse, MemCubeError,
    MemoryResponse, SchedulerStatusResponse, SearchResponse, UpdateMemoryRequest,
    UpdateMemoryResponse,
};
use serde::Deserialize;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

/// In-memory implementation of AuditStore (process lifetime only).
pub struct InMemoryAuditStore {
    events: tokio::sync::RwLock<Vec<AuditEvent>>,
}

impl InMemoryAuditStore {
    pub fn new() -> Self {
        Self {
            events: tokio::sync::RwLock::new(Vec::new()),
        }
    }
}

impl Default for InMemoryAuditStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl AuditStore for InMemoryAuditStore {
    async fn append(&self, event: AuditEvent) -> Result<(), mem_types::AuditStoreError> {
        self.events.write().await.push(event);
        Ok(())
    }

    async fn list(
        &self,
        opts: &AuditListOptions,
    ) -> Result<Vec<AuditEvent>, mem_types::AuditStoreError> {
        let guard = self.events.read().await;
        let mut out: Vec<AuditEvent> = guard.iter().cloned().collect();
        apply_audit_list_opts(&mut out, opts);
        Ok(out)
    }
}

/// JSONL file-backed AuditStore (persists across restarts).
pub struct JsonlAuditStore {
    path: std::path::PathBuf,
    append_lock: tokio::sync::Mutex<()>,
}

impl JsonlAuditStore {
    pub fn new(path: impl AsRef<std::path::Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            append_lock: tokio::sync::Mutex::new(()),
        }
    }
}

#[async_trait::async_trait]
impl AuditStore for JsonlAuditStore {
    async fn append(&self, event: AuditEvent) -> Result<(), mem_types::AuditStoreError> {
        let _guard = self.append_lock.lock().await;
        let line = serde_json::to_string(&event)
            .map_err(|e| mem_types::AuditStoreError::Other(e.to_string()))?;
        let mut f = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|e| mem_types::AuditStoreError::Other(e.to_string()))?;
        f.write_all(format!("{}\n", line).as_bytes())
            .await
            .map_err(|e| mem_types::AuditStoreError::Other(e.to_string()))?;
        Ok(())
    }

    async fn list(
        &self,
        opts: &AuditListOptions,
    ) -> Result<Vec<AuditEvent>, mem_types::AuditStoreError> {
        let content = match tokio::fs::read_to_string(&self.path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(mem_types::AuditStoreError::Other(e.to_string())),
        };
        let mut out: Vec<AuditEvent> = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(ev) = serde_json::from_str(line) {
                out.push(ev);
            }
        }
        apply_audit_list_opts(&mut out, opts);
        Ok(out)
    }
}

fn apply_audit_list_opts(out: &mut Vec<AuditEvent>, opts: &AuditListOptions) {
    if let Some(ref uid) = opts.user_id {
        out.retain(|e| &e.user_id == uid);
    }
    if let Some(ref cid) = opts.cube_id {
        out.retain(|e| &e.cube_id == cid);
    }
    if let Some(ref since) = opts.since {
        out.retain(|e| e.timestamp.as_str() >= since.as_str());
    }
    out.reverse();
    let offset = opts.offset.unwrap_or(0) as usize;
    let limit = opts.limit.unwrap_or(100) as usize;
    let taken: Vec<AuditEvent> = std::mem::take(out)
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect();
    *out = taken;
}

pub struct AppState {
    pub cube: Arc<dyn MemCube + Send + Sync>,
    pub scheduler: Arc<dyn Scheduler + Send + Sync>,
    pub audit_log: Arc<dyn AuditStore + Send + Sync>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/product/add", post(handle_add))
        .route("/product/search", post(handle_search))
        .route("/product/scheduler/status", get(handle_scheduler_status))
        .route("/product/update_memory", post(handle_update_memory))
        .route("/product/delete_memory", post(handle_delete_memory))
        .route("/product/get_memory", post(handle_get_memory))
        .route("/product/audit/list", get(handle_audit_list))
        .route("/health", get(handle_health))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn push_audit(state: &AppState, event: AuditEvent) {
    let _ = state.audit_log.append(event).await;
}

async fn handle_add(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ApiAddRequest>,
) -> Json<MemoryResponse> {
    if req.async_mode.as_str() == "async" {
        match state.scheduler.submit_add(req).await {
            Ok(task_id) => {
                tracing::info!(task_id = %task_id, "add job submitted (async)");
                Json(MemoryResponse {
                    code: 200,
                    message: "Memory add job submitted".to_string(),
                    data: Some(vec![serde_json::json!({ "task_id": task_id })]),
                })
            }
            Err(e) => Json(MemoryResponse {
                code: 500,
                message: e.to_string(),
                data: None,
            }),
        }
    } else {
        let cube_ids = req.writable_cube_ids();
        let user_id = req.user_id.clone();
        let cube_id = cube_ids.first().cloned().unwrap_or_else(|| user_id.clone());
        match state.cube.add_memories(&req).await {
            Ok(res) => {
                let memory_id = res
                    .data
                    .as_ref()
                    .and_then(|d| d.first())
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                push_audit(
                    &state,
                    AuditEvent {
                        event_id: Uuid::new_v4().to_string(),
                        kind: AuditEventKind::Add,
                        memory_id,
                        user_id,
                        cube_id,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        input_summary: None,
                        outcome: Some(format!("code={}", res.code)),
                    },
                )
                .await;
                Json(res)
            }
            Err(e) => Json(MemoryResponse {
                code: 500,
                message: e.to_string(),
                data: None,
            }),
        }
    }
}

async fn handle_search(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ApiSearchRequest>,
) -> Json<SearchResponse> {
    match state.cube.search_memories(&req).await {
        Ok(res) => Json(res),
        Err(e) => Json(SearchResponse {
            code: 500,
            message: e.to_string(),
            data: None,
        }),
    }
}

#[derive(Debug, Deserialize)]
pub struct SchedulerStatusQuery {
    pub user_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
}

async fn handle_scheduler_status(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SchedulerStatusQuery>,
) -> Json<SchedulerStatusResponse> {
    let task_id = match &q.task_id {
        Some(t) => t.as_str(),
        None => {
            return Json(SchedulerStatusResponse {
                code: 400,
                message: "task_id is required".to_string(),
                data: None,
            });
        }
    };
    match state.scheduler.get_status(&q.user_id, task_id).await {
        Ok(Some(job)) => Json(SchedulerStatusResponse {
            code: 200,
            message: "Success".to_string(),
            data: Some(job),
        }),
        Ok(None) => Json(SchedulerStatusResponse {
            code: 404,
            message: "Job not found".to_string(),
            data: None,
        }),
        Err(e) => Json(SchedulerStatusResponse {
            code: 500,
            message: e.to_string(),
            data: None,
        }),
    }
}

async fn handle_update_memory(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpdateMemoryRequest>,
) -> Json<UpdateMemoryResponse> {
    let user_id = req.user_id.clone();
    let cube_id = req
        .mem_cube_id
        .clone()
        .unwrap_or_else(|| req.user_id.clone());
    let memory_id = req.memory_id.clone();
    match state.cube.update_memory(&req).await {
        Ok(res) => {
            push_audit(
                &state,
                AuditEvent {
                    event_id: Uuid::new_v4().to_string(),
                    kind: AuditEventKind::Update,
                    memory_id: Some(memory_id),
                    user_id,
                    cube_id,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    input_summary: None,
                    outcome: Some(format!("code={}", res.code)),
                },
            )
            .await;
            Json(res)
        }
        Err(MemCubeError::NotFound(msg)) => Json(UpdateMemoryResponse {
            code: 404,
            message: msg,
            data: None,
        }),
        Err(e) => Json(UpdateMemoryResponse {
            code: 500,
            message: e.to_string(),
            data: None,
        }),
    }
}

async fn handle_delete_memory(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ForgetMemoryRequest>,
) -> Json<ForgetMemoryResponse> {
    let user_id = req.user_id.clone();
    let cube_id = req
        .mem_cube_id
        .clone()
        .unwrap_or_else(|| req.user_id.clone());
    let memory_id = req.memory_id.clone();
    match state.cube.forget_memory(&req).await {
        Ok(res) => {
            push_audit(
                &state,
                AuditEvent {
                    event_id: Uuid::new_v4().to_string(),
                    kind: AuditEventKind::Forget,
                    memory_id: Some(memory_id),
                    user_id,
                    cube_id,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    input_summary: None,
                    outcome: Some(format!("code={}", res.code)),
                },
            )
            .await;
            Json(res)
        }
        Err(MemCubeError::NotFound(msg)) => Json(ForgetMemoryResponse {
            code: 404,
            message: msg,
            data: None,
        }),
        Err(e) => Json(ForgetMemoryResponse {
            code: 500,
            message: e.to_string(),
            data: None,
        }),
    }
}

async fn handle_get_memory(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GetMemoryRequest>,
) -> Json<GetMemoryResponse> {
    match state.cube.get_memory(&req).await {
        Ok(res) => Json(res),
        Err(e) => Json(GetMemoryResponse {
            code: 500,
            message: e.to_string(),
            data: None,
        }),
    }
}

#[derive(Debug, Deserialize)]
pub struct AuditListQuery {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub cube_id: Option<String>,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

async fn handle_audit_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AuditListQuery>,
) -> Json<AuditListResponse> {
    let opts = AuditListOptions {
        user_id: q.user_id,
        cube_id: q.cube_id,
        since: q.since,
        limit: q.limit,
        offset: q.offset,
    };
    match state.audit_log.list(&opts).await {
        Ok(events) => Json(AuditListResponse {
            code: 200,
            message: "Success".to_string(),
            data: Some(events),
        }),
        Err(e) => Json(AuditListResponse {
            code: 500,
            message: e.to_string(),
            data: None,
        }),
    }
}

#[derive(Debug, serde::Serialize)]
pub struct AuditListResponse {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<AuditEvent>>,
}

async fn handle_health() -> &'static str {
    "ok"
}
