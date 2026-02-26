//! Axum server and routes.

use axum::{
    extract::Extension,
    extract::MatchedPath,
    extract::{Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use mem_scheduler::Scheduler;
use mem_types::MemCube;
use mem_types::{
    ApiAddRequest, ApiHybridSearchRequest, ApiSearchRequest, AuditEvent, AuditEventKind,
    AuditListOptions, AuditStore, Entity, EntityRelationType, EntityType, ForgetMemoryRequest,
    ForgetMemoryResponse, GetMemoryRequest, GetMemoryResponse, GraphNeighborsRequest,
    GraphNeighborsResponse, GraphPathRequest, GraphPathResponse, GraphPathsRequest,
    GraphPathsResponse, HybridSearchResponse, MemCubeError, MemoryResponse,
    SchedulerStatusResponse, SearchResponse, UpdateMemoryRequest, UpdateMemoryResponse,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

// Re-export entity types for use in routes
pub use mem_types::{
    Entity as EntityType_, EntityRelationType as EntityRelationType_, EntityType as EntityTypeEnum,
};

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
    pub auth_token: Option<String>,
}

#[derive(Clone)]
struct RequestMeta {
    request_id: String,
}

struct ApiMetrics {
    inner: Mutex<ApiMetricsInner>,
}

impl ApiMetrics {
    fn new() -> Self {
        Self {
            inner: Mutex::new(ApiMetricsInner {
                requests_total: HashMap::new(),
                errors_total: HashMap::new(),
                request_duration_ms: HashMap::new(),
                duration_buckets_ms: vec![5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000],
            }),
        }
    }

    fn observe(&self, endpoint: String, method: String, status: u16, duration_ms: f64) {
        let mut inner = self.inner.lock().expect("metrics mutex poisoned");
        let req_key = RequestMetricKey {
            endpoint: endpoint.clone(),
            method: method.clone(),
            status,
        };
        *inner.requests_total.entry(req_key.clone()).or_insert(0) += 1;
        if status >= 400 {
            *inner.errors_total.entry(req_key).or_insert(0) += 1;
        }
        let lat_key = LatencyMetricKey { endpoint, method };
        let bucket_bounds = inner.duration_buckets_ms.clone();
        let entry = inner
            .request_duration_ms
            .entry(lat_key)
            .or_insert_with(|| LatencyMetric::new(bucket_bounds.len()));
        entry.observe(duration_ms, &bucket_bounds);
    }

    fn render_prometheus(&self) -> String {
        let inner = self.inner.lock().expect("metrics mutex poisoned");
        let mut out = String::new();

        out.push_str("# HELP mem_api_requests_total Total HTTP requests\n");
        out.push_str("# TYPE mem_api_requests_total counter\n");
        for (k, v) in &inner.requests_total {
            out.push_str(&format!(
                "mem_api_requests_total{{endpoint=\"{}\",method=\"{}\",status=\"{}\"}} {}\n",
                escape_label(&k.endpoint),
                escape_label(&k.method),
                k.status,
                v
            ));
        }

        out.push_str("# HELP mem_api_errors_total Total HTTP error responses\n");
        out.push_str("# TYPE mem_api_errors_total counter\n");
        for (k, v) in &inner.errors_total {
            out.push_str(&format!(
                "mem_api_errors_total{{endpoint=\"{}\",method=\"{}\",status=\"{}\"}} {}\n",
                escape_label(&k.endpoint),
                escape_label(&k.method),
                k.status,
                v
            ));
        }

        out.push_str("# HELP mem_api_request_duration_ms HTTP request latency in milliseconds\n");
        out.push_str("# TYPE mem_api_request_duration_ms histogram\n");
        for (k, v) in &inner.request_duration_ms {
            let mut cumulative = 0u64;
            for (idx, bucket_count) in v.buckets.iter().enumerate() {
                cumulative += *bucket_count;
                out.push_str(&format!(
                    "mem_api_request_duration_ms_bucket{{endpoint=\"{}\",method=\"{}\",le=\"{}\"}} {}\n",
                    escape_label(&k.endpoint),
                    escape_label(&k.method),
                    inner.duration_buckets_ms[idx],
                    cumulative
                ));
            }
            out.push_str(&format!(
                "mem_api_request_duration_ms_bucket{{endpoint=\"{}\",method=\"{}\",le=\"+Inf\"}} {}\n",
                escape_label(&k.endpoint),
                escape_label(&k.method),
                v.count
            ));
            out.push_str(&format!(
                "mem_api_request_duration_ms_sum{{endpoint=\"{}\",method=\"{}\"}} {:.6}\n",
                escape_label(&k.endpoint),
                escape_label(&k.method),
                v.sum
            ));
            out.push_str(&format!(
                "mem_api_request_duration_ms_count{{endpoint=\"{}\",method=\"{}\"}} {}\n",
                escape_label(&k.endpoint),
                escape_label(&k.method),
                v.count
            ));
        }

        out
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RequestMetricKey {
    endpoint: String,
    method: String,
    status: u16,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct LatencyMetricKey {
    endpoint: String,
    method: String,
}

#[derive(Clone, Debug)]
struct LatencyMetric {
    buckets: Vec<u64>,
    count: u64,
    sum: f64,
}

impl LatencyMetric {
    fn new(bucket_len: usize) -> Self {
        Self {
            buckets: vec![0; bucket_len],
            count: 0,
            sum: 0.0,
        }
    }

    fn observe(&mut self, duration_ms: f64, bucket_bounds: &[u64]) {
        self.count += 1;
        self.sum += duration_ms;
        for (idx, upper) in bucket_bounds.iter().enumerate() {
            if duration_ms <= *upper as f64 {
                self.buckets[idx] += 1;
                return;
            }
        }
    }
}

struct ApiMetricsInner {
    requests_total: HashMap<RequestMetricKey, u64>,
    errors_total: HashMap<RequestMetricKey, u64>,
    request_duration_ms: HashMap<LatencyMetricKey, LatencyMetric>,
    duration_buckets_ms: Vec<u64>,
}

fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

static METRICS: OnceLock<ApiMetrics> = OnceLock::new();

fn metrics() -> &'static ApiMetrics {
    METRICS.get_or_init(ApiMetrics::new)
}

fn error_log_sample_rate() -> f64 {
    std::env::var("MEMOS_ERROR_LOG_SAMPLE_RATE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|v| v.clamp(0.0, 1.0))
        .unwrap_or(0.1)
}

fn should_sample(request_id: &str, rate: f64) -> bool {
    if rate <= 0.0 {
        return false;
    }
    if rate >= 1.0 {
        return true;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    request_id.hash(&mut h);
    let bucket = h.finish() % 10_000;
    bucket < (rate * 10_000.0) as u64
}

pub fn router(state: Arc<AppState>) -> Router {
    let product_routes = Router::new()
        .route("/product/add", post(handle_add))
        .route("/product/search", post(handle_search))
        .route("/product/hybrid_search", post(handle_hybrid_search))
        .route("/product/scheduler/status", get(handle_scheduler_status))
        .route("/product/update_memory", post(handle_update_memory))
        .route("/product/delete_memory", post(handle_delete_memory))
        .route("/product/get_memory", post(handle_get_memory))
        .route("/product/graph/neighbors", post(handle_graph_neighbors))
        .route("/product/graph/path", post(handle_graph_path))
        .route("/product/graph/paths", post(handle_graph_paths))
        .route("/product/audit/list", get(handle_audit_list))
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_auth,
        ));

    Router::new()
        .route("/health", get(handle_health))
        .route("/metrics", get(handle_metrics))
        .merge(product_routes)
        .layer(middleware::from_fn(request_id_middleware))
        .layer(middleware::from_fn(metrics_middleware))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn request_id_middleware(mut req: Request, next: Next) -> Response {
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    req.extensions_mut().insert(RequestMeta {
        request_id: request_id.clone(),
    });
    let mut response = next.run(req).await;
    if let Ok(hv) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", hv);
    }
    response
}

async fn metrics_middleware(req: Request, next: Next) -> Response {
    if req.uri().path() == "/metrics" {
        return next.run(req).await;
    }
    let method = req.method().to_string();
    let endpoint = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let started = Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16();
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;

    let endpoint_for_log = endpoint.clone();
    let method_for_log = method.clone();
    metrics().observe(endpoint, method, status, duration_ms);
    if status >= 400 {
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let sample_rate = error_log_sample_rate();
        if should_sample(request_id, sample_rate) {
            tracing::warn!(
                endpoint = %endpoint_for_log,
                method = %method_for_log,
                request_id = %request_id,
                status = status,
                duration_ms = duration_ms,
                sample_rate = sample_rate,
                "sampled api error"
            );
        }
    }
    response
}

async fn require_auth(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let Some(expected) = state.auth_token.as_ref() else {
        return next.run(request).await;
    };
    let authorized = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|token| token == expected)
        .unwrap_or(false);
    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "code": 401,
                "message": "Unauthorized",
                "data": serde_json::Value::Null
            })),
        )
            .into_response()
    }
}

async fn push_audit(state: &AppState, event: AuditEvent) {
    let _ = state.audit_log.append(event).await;
}

async fn handle_add(
    State(state): State<Arc<AppState>>,
    Extension(req_meta): Extension<RequestMeta>,
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
                        input_summary: Some(format!("request_id={}", req_meta.request_id)),
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

async fn handle_hybrid_search(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ApiHybridSearchRequest>,
) -> (StatusCode, Json<HybridSearchResponse>) {
    match state.cube.hybrid_search(&req).await {
        Ok(res) => (StatusCode::OK, Json(res)),
        Err(e) => {
            let msg = e.to_string();
            let (code, response) = if msg.contains("not supported") {
                (
                    StatusCode::NOT_IMPLEMENTED,
                    HybridSearchResponse {
                        code: 501,
                        message: msg,
                        data: None,
                    },
                )
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    HybridSearchResponse {
                        code: 500,
                        message: msg,
                        data: None,
                    },
                )
            };
            (code, Json(response))
        }
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
    Extension(req_meta): Extension<RequestMeta>,
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
                    input_summary: Some(format!("request_id={}", req_meta.request_id)),
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
    Extension(req_meta): Extension<RequestMeta>,
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
                    input_summary: Some(format!("request_id={}", req_meta.request_id)),
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

async fn handle_graph_neighbors(
    State(state): State<Arc<AppState>>,
    Extension(req_meta): Extension<RequestMeta>,
    Json(req): Json<GraphNeighborsRequest>,
) -> Json<GraphNeighborsResponse> {
    let started = Instant::now();
    let response = match state.cube.graph_neighbors(&req).await {
        Ok(res) => Json(res),
        Err(MemCubeError::BadRequest(msg)) => Json(GraphNeighborsResponse {
            code: 400,
            message: msg,
            data: None,
        }),
        Err(MemCubeError::NotFound(msg)) => Json(GraphNeighborsResponse {
            code: 404,
            message: msg,
            data: None,
        }),
        Err(e) => Json(GraphNeighborsResponse {
            code: 500,
            message: e.to_string(),
            data: None,
        }),
    };
    let items = response.0.data.as_ref().map(|d| d.items.len()).unwrap_or(0);
    tracing::info!(
        endpoint = "/product/graph/neighbors",
        request_id = %req_meta.request_id,
        memory_id = %req.memory_id,
        user_id = %req.user_id,
        code = response.0.code,
        items = items,
        duration_ms = started.elapsed().as_millis(),
        "graph neighbors handled"
    );
    response
}

async fn handle_graph_path(
    State(state): State<Arc<AppState>>,
    Extension(req_meta): Extension<RequestMeta>,
    Json(req): Json<GraphPathRequest>,
) -> Json<GraphPathResponse> {
    let started = Instant::now();
    let response = match state.cube.graph_path(&req).await {
        Ok(res) => Json(res),
        Err(MemCubeError::BadRequest(msg)) => Json(GraphPathResponse {
            code: 400,
            message: msg,
            data: None,
        }),
        Err(MemCubeError::NotFound(msg)) => Json(GraphPathResponse {
            code: 404,
            message: msg,
            data: None,
        }),
        Err(e) => Json(GraphPathResponse {
            code: 500,
            message: e.to_string(),
            data: None,
        }),
    };
    let hops = response.0.data.as_ref().map(|d| d.hops).unwrap_or(0);
    tracing::info!(
        endpoint = "/product/graph/path",
        request_id = %req_meta.request_id,
        source_memory_id = %req.source_memory_id,
        target_memory_id = %req.target_memory_id,
        user_id = %req.user_id,
        code = response.0.code,
        hops = hops,
        duration_ms = started.elapsed().as_millis(),
        "graph path handled"
    );
    response
}

async fn handle_graph_paths(
    State(state): State<Arc<AppState>>,
    Extension(req_meta): Extension<RequestMeta>,
    Json(req): Json<GraphPathsRequest>,
) -> Json<GraphPathsResponse> {
    let started = Instant::now();
    let response = match state.cube.graph_paths(&req).await {
        Ok(res) => Json(res),
        Err(MemCubeError::BadRequest(msg)) => Json(GraphPathsResponse {
            code: 400,
            message: msg,
            data: None,
        }),
        Err(MemCubeError::NotFound(msg)) => Json(GraphPathsResponse {
            code: 404,
            message: msg,
            data: None,
        }),
        Err(e) => Json(GraphPathsResponse {
            code: 500,
            message: e.to_string(),
            data: None,
        }),
    };
    let path_count = response.0.data.as_ref().map(|d| d.len()).unwrap_or(0);
    tracing::info!(
        endpoint = "/product/graph/paths",
        request_id = %req_meta.request_id,
        source_memory_id = %req.source_memory_id,
        target_memory_id = %req.target_memory_id,
        user_id = %req.user_id,
        top_k_paths = req.top_k_paths,
        code = response.0.code,
        path_count = path_count,
        duration_ms = started.elapsed().as_millis(),
        "graph paths handled"
    );
    response
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

async fn handle_metrics() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4".to_string(),
        )],
        metrics().render_prometheus(),
    )
        .into_response()
}

// ============================================================================
// Entity Routes (Simplified - require EntityAwareMemCube)
// ============================================================================

/// Get entity by ID request.
#[derive(Debug, Deserialize)]
pub struct GetEntityRequest {
    pub entity_id: String,
}

/// Get entity by name request.
#[derive(Debug, Deserialize)]
pub struct GetEntityByNameRequest {
    pub name: String,
}

/// Search entities request.
#[derive(Debug, Deserialize)]
pub struct SearchEntitiesRequest {
    pub query: String,
    #[serde(default)]
    pub entity_type: Option<EntityType>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub fuzzy: bool,
}

fn default_limit() -> u32 {
    20
}

/// List entities by type request.
#[derive(Debug, Deserialize)]
pub struct ListEntitiesByTypeRequest {
    pub entity_type: EntityType,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Get entity relations request.
#[derive(Debug, Deserialize)]
pub struct GetEntityRelationsRequest {
    pub entity_id: String,
    #[serde(default)]
    pub relation_type: Option<EntityRelationType>,
    #[serde(default = "default_related_limit")]
    pub limit: u32,
}

fn default_related_limit() -> u32 {
    20
}

/// Get entities for memory request.
#[derive(Debug, Deserialize)]
pub struct GetMemoryEntitiesRequest {
    pub memory_id: String,
}

/// Entity response.
#[derive(Debug, serde::Serialize)]
pub struct EntityApiResponse {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// Entity list response.
#[derive(Debug, serde::Serialize)]
pub struct EntityListApiResponse {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub entities: Vec<serde_json::Value>,
    pub total_count: u32,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Entity relations response.
#[derive(Debug, serde::Serialize)]
pub struct EntityRelationsApiResponse {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub relations: Vec<RelationItem>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct RelationItem {
    pub relation_type: String,
    pub entity: EntityItem,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct EntityItem {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub attributes: Option<serde_json::Value>,
    pub occurrence_count: u32,
    pub confidence: f64,
}

/// Entity stats response.
#[derive(Debug, serde::Serialize)]
pub struct EntityStatsApiResponse {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<EntityStatsData>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct EntityStatsData {
    pub total_entities: u32,
    pub total_relations: u32,
    #[serde(default)]
    pub type_counts: serde_json::Value,
}

/// Convert internal Entity to API format.
fn entity_to_api(entity: &Entity) -> serde_json::Value {
    serde_json::json!({
        "id": entity.id,
        "name": entity.name,
        "entity_type": entity.entity_type.to_string(),
        "description": entity.description,
        "attributes": entity.attributes,
        "memory_ids": entity.memory_ids,
        "name_variants": entity.name_variants,
        "occurrence_count": entity.metadata.occurrence_count,
        "confidence": entity.metadata.confidence,
        "first_seen": entity.metadata.first_seen,
        "last_updated": entity.metadata.last_updated,
        "version": entity.version,
    })
}
