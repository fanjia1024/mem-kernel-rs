//! Request and response DTOs compatible with MemOS product API.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    EntityRelationType, EntityType, FusionWeights, GraphSearchConfig, HybridSearchMode,
    KeywordSearchConfig, RerankConfig, SearchChannel,
};

/// Single chat message (user/assistant).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Add-memory request (MemOS APIADDRequest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiAddRequest {
    pub user_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub writable_cube_ids: Option<Vec<String>>,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    #[serde(default = "default_async_mode")]
    pub async_mode: String,
    #[serde(default)]
    pub messages: Option<Vec<Message>>,
    #[serde(default)]
    pub memory_content: Option<String>,
    #[serde(default)]
    pub chat_history: Option<Vec<Message>>,
    #[serde(default)]
    pub custom_tags: Option<Vec<String>>,
    #[serde(default)]
    pub info: Option<HashMap<String, serde_json::Value>>,
    /// Optional graph relations to existing memories while adding this new memory.
    #[serde(default)]
    pub relations: Option<Vec<AddMemoryRelation>>,
    #[serde(default)]
    pub is_feedback: bool,
}

fn default_async_mode() -> String {
    "sync".to_string()
}

impl ApiAddRequest {
    /// Resolve cube ids to write to: writable_cube_ids or [user_id].
    pub fn writable_cube_ids(&self) -> Vec<String> {
        if let Some(ref ids) = self.writable_cube_ids {
            if !ids.is_empty() {
                return ids.clone();
            }
        }
        if let Some(ref id) = self.mem_cube_id {
            return vec![id.clone()];
        }
        vec![self.user_id.clone()]
    }

    /// Content to store: from messages or memory_content.
    pub fn content_to_store(&self) -> Option<String> {
        if let Some(ref msgs) = self.messages {
            if !msgs.is_empty() {
                let parts: Vec<String> = msgs
                    .iter()
                    .map(|m| format!("{}: {}", m.role, m.content))
                    .collect();
                return Some(parts.join("\n"));
            }
        }
        self.memory_content.clone()
    }
}

/// Time range filter for search (P0: new)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    /// Start of time range (ISO8601)
    pub start: String,
    /// End of time range (ISO8601)
    pub end: String,
}

/// Search-memory request (MemOS APISearchRequest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiSearchRequest {
    pub query: String,
    pub user_id: String,
    #[serde(default)]
    pub readable_cube_ids: Option<Vec<String>>,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    #[serde(default = "default_top_k")]
    pub top_k: u32,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub relativity: f64,
    #[serde(default)]
    pub include_preference: bool,
    #[serde(default)]
    pub pref_top_k: u32,
    #[serde(default)]
    pub filter: Option<HashMap<String, serde_json::Value>>,
    /// Search within time range (P0: new)
    #[serde(default)]
    pub time_range: Option<TimeRange>,
    /// Only return memories created after this time (ISO8601)
    #[serde(default)]
    pub since: Option<String>,
    /// Only return memories created before this time (ISO8601)
    #[serde(default)]
    pub until: Option<String>,
}

fn default_top_k() -> u32 {
    10
}

impl ApiSearchRequest {
    /// Resolve cube ids to read from: readable_cube_ids or mem_cube_id or [user_id].
    pub fn readable_cube_ids(&self) -> Vec<String> {
        if let Some(ref ids) = self.readable_cube_ids {
            if !ids.is_empty() {
                return ids.clone();
            }
        }
        if let Some(ref id) = self.mem_cube_id {
            return vec![id.clone()];
        }
        vec![self.user_id.clone()]
    }
}

/// Base response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseResponse<T> {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<T>,
}

fn default_code() -> i32 {
    200
}

/// Add-memory response (MemOS MemoryResponse).
pub type MemoryResponse = BaseResponse<Vec<serde_json::Value>>;

/// Single memory item as returned in search (id, memory, metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: String,
    pub memory: String,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// One bucket of memories (e.g. WorkingMemory, LongTermMemory).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBucket {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub memories: Vec<MemoryItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_nodes: Option<usize>,
}

/// Search result data: text_mem and optional pref_mem.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchResponseData {
    #[serde(default)]
    pub text_mem: Vec<MemoryBucket>,
    #[serde(default)]
    pub pref_mem: Vec<MemoryBucket>,
}

/// Search response (MemOS SearchResponse).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<SearchResponseData>,
}

/// Request to update an existing memory (partial fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMemoryRequest {
    pub memory_id: String,
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Request to forget (soft or hard delete) a memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgetMemoryRequest {
    pub memory_id: String,
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    /// If true, soft delete (mark tombstone); else hard delete.
    #[serde(default)]
    pub soft: bool,
}

/// Response for update_memory / forget_memory (same envelope as add).
pub type UpdateMemoryResponse = BaseResponse<Vec<serde_json::Value>>;
pub type ForgetMemoryResponse = BaseResponse<Vec<serde_json::Value>>;

/// Optional relation spec used by add-memory request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddMemoryRelation {
    /// Existing memory id to connect with the newly added memory.
    pub memory_id: String,
    pub relation: String,
    /// Edge direction relative to the newly added memory node.
    /// `outbound`: new -> memory_id; `inbound`: memory_id -> new; `both`: write both edges.
    #[serde(default)]
    pub direction: GraphDirection,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Request to get a single memory by id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMemoryRequest {
    pub memory_id: String,
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    /// If true, return memories marked tombstone (soft-deleted). Default false.
    #[serde(default)]
    pub include_deleted: bool,
}

/// Response for get_memory: optional MemoryItem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMemoryResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<MemoryItem>,
}

/// Internal memory node (id, memory, metadata, optional embedding).
#[derive(Debug, Clone)]
pub struct MemoryNode {
    pub id: String,
    pub memory: String,
    pub metadata: HashMap<String, serde_json::Value>,
    pub embedding: Option<Vec<f32>>,
}

/// Graph edge between two memory nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub relation: String,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Neighbor item for graph traversal response.
#[derive(Debug, Clone)]
pub struct GraphNeighbor {
    pub edge: MemoryEdge,
    pub node: MemoryNode,
}

/// API request for graph neighbor query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNeighborsRequest {
    pub memory_id: String,
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    #[serde(default)]
    pub relation: Option<String>,
    #[serde(default)]
    pub direction: GraphDirection,
    #[serde(default = "default_graph_limit")]
    pub limit: u32,
    /// Opaque cursor token from previous response for pagination.
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub include_embedding: bool,
    #[serde(default)]
    pub include_deleted: bool,
}

fn default_graph_limit() -> u32 {
    10
}

/// API response item for one graph neighbor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNeighborItem {
    pub edge: MemoryEdge,
    pub memory: MemoryItem,
}

/// API response payload for graph neighbor query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNeighborsData {
    pub items: Vec<GraphNeighborItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// API response for graph neighbor query.
pub type GraphNeighborsResponse = BaseResponse<GraphNeighborsData>;

/// Internal shortest-path result.
#[derive(Debug, Clone)]
pub struct GraphPath {
    pub node_ids: Vec<String>,
    pub edges: Vec<MemoryEdge>,
}

/// API request for graph path query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPathRequest {
    pub source_memory_id: String,
    pub target_memory_id: String,
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    #[serde(default)]
    pub relation: Option<String>,
    #[serde(default)]
    pub direction: GraphDirection,
    #[serde(default = "default_graph_max_depth")]
    pub max_depth: u32,
    #[serde(default)]
    pub include_deleted: bool,
}

fn default_graph_max_depth() -> u32 {
    6
}

/// API response payload for graph path query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPathData {
    pub hops: u32,
    pub nodes: Vec<MemoryItem>,
    pub edges: Vec<MemoryEdge>,
}

/// API response for graph path query.
pub type GraphPathResponse = BaseResponse<GraphPathData>;

/// API request for multi-path graph query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPathsRequest {
    pub source_memory_id: String,
    pub target_memory_id: String,
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    #[serde(default)]
    pub relation: Option<String>,
    #[serde(default)]
    pub direction: GraphDirection,
    #[serde(default = "default_graph_max_depth")]
    pub max_depth: u32,
    #[serde(default = "default_graph_top_k_paths")]
    pub top_k_paths: u32,
    #[serde(default)]
    pub include_deleted: bool,
}

fn default_graph_top_k_paths() -> u32 {
    3
}

/// API response for multi-path query.
pub type GraphPathsResponse = BaseResponse<Vec<GraphPathData>>;

/// Traversal direction for graph neighbor query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphDirection {
    #[default]
    Outbound,
    Inbound,
    Both,
}

/// Scope for memory (MemOS: WorkingMemory, LongTermMemory, UserMemory).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryScope {
    WorkingMemory,
    LongTermMemory,
    UserMemory,
}

impl MemoryScope {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryScope::WorkingMemory => "WorkingMemory",
            MemoryScope::LongTermMemory => "LongTermMemory",
            MemoryScope::UserMemory => "UserMemory",
        }
    }
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================================
// Hybrid Search API Types (Phase 6)
// ============================================================================

/// Hybrid search request combining vector, keyword, and graph channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiHybridSearchRequest {
    /// User ID for authorization.
    pub user_id: String,
    /// Cube IDs to search in.
    #[serde(default)]
    pub readable_cube_ids: Option<Vec<String>>,
    /// Query text for search.
    pub query: String,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    pub top_k: u32,
    /// Score threshold filter (0.0 - 1.0).
    #[serde(default)]
    pub relativity: Option<f64>,
    /// Search mode.
    #[serde(default)]
    pub mode: HybridSearchMode,
    /// Weights for score fusion.
    #[serde(default)]
    pub fusion_weights: Option<FusionWeights>,
    /// Keyword search configuration.
    #[serde(default)]
    pub keyword_config: Option<KeywordSearchConfig>,
    /// Graph search configuration.
    #[serde(default)]
    pub graph_config: Option<GraphSearchConfig>,
    /// Reranking configuration.
    #[serde(default)]
    pub rerank_config: Option<RerankConfig>,
}

impl ApiHybridSearchRequest {
    /// Get searchable cube IDs.
    pub fn readable_cube_ids(&self) -> Vec<String> {
        if let Some(ref ids) = self.readable_cube_ids {
            if !ids.is_empty() {
                return ids.clone();
            }
        }
        vec![self.user_id.clone()]
    }
}

/// Result from a single search channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelResult {
    /// Channel type.
    pub channel: SearchChannel,
    /// Number of results from this channel.
    pub count: u32,
    /// Raw hits from this channel.
    #[serde(default)]
    pub hits: Vec<serde_json::Value>,
}

/// A single hit from hybrid search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchHit {
    /// Memory ID.
    pub memory_id: String,
    /// Memory content.
    pub memory_content: String,
    /// Memory metadata.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Vector similarity score (raw).
    #[serde(default)]
    pub vector_score: Option<f64>,
    /// Keyword/BM25 score (raw).
    #[serde(default)]
    pub keyword_score: Option<f64>,
    /// Graph score (raw).
    #[serde(default)]
    pub graph_score: Option<f64>,
    /// Fused normalized score (0.0 - 1.0).
    pub fused_score: f64,
    /// Normalized vector score.
    #[serde(default)]
    pub vector_norm: Option<f64>,
    /// Normalized keyword score.
    #[serde(default)]
    pub keyword_norm: Option<f64>,
    /// Normalized graph score.
    #[serde(default)]
    pub graph_norm: Option<f64>,
    /// Rerank score (if enabled).
    #[serde(default)]
    pub rerank_score: Option<f64>,
    /// Source channels that contributed to this hit.
    #[serde(default)]
    pub channels: Vec<SearchChannel>,
}

/// Hybrid search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<HybridSearchData>,
}

impl Default for HybridSearchResponse {
    fn default() -> Self {
        Self {
            code: 200,
            message: String::new(),
            data: None,
        }
    }
}

/// Hybrid search data payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchData {
    /// Original query.
    pub query: String,
    /// Total hits before reranking.
    pub total_candidates: u32,
    /// Final hits after fusion/reranking.
    pub hits: Vec<HybridSearchHit>,
    /// Results from individual channels.
    #[serde(default)]
    pub channel_results: Vec<ChannelResult>,
    /// Whether reranking was applied.
    pub rerank_used: bool,
    /// Processing latency in milliseconds.
    pub latency_ms: u64,
}

/// Entity search request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEntitiesRequest {
    /// Search query.
    pub query: String,
    /// Filter by entity type.
    #[serde(default)]
    pub entity_type: Option<EntityType>,
    /// Maximum results to return.
    #[serde(default = "default_entity_limit")]
    pub limit: u32,
    /// Enable fuzzy matching.
    #[serde(default)]
    pub fuzzy: bool,
}

fn default_entity_limit() -> u32 {
    20
}

/// Get entity by ID request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetEntityRequest {
    pub entity_id: String,
}

/// List entities by type request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListEntitiesByTypeRequest {
    pub entity_type: EntityType,
    #[serde(default = "default_entity_limit")]
    pub limit: u32,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Entity search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEntitiesResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<SearchEntitiesData>,
}

/// Entity search result data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEntitiesData {
    /// Matching entities.
    pub entities: Vec<serde_json::Value>,
    /// Total count.
    pub total_count: u32,
    /// Optional cursor for pagination.
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Entity-aware search request (search memories by entity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityAwareSearchRequest {
    /// Entity name to search for.
    pub entity_name: String,
    /// Filter by entity type.
    #[serde(default)]
    pub entity_type: Option<EntityType>,
    /// User ID.
    pub user_id: String,
    /// Cube IDs.
    #[serde(default)]
    pub readable_cube_ids: Option<Vec<String>>,
    /// Maximum memories per entity.
    #[serde(default = "default_memories_per_entity")]
    pub memories_per_entity: u32,
    /// Maximum total results.
    #[serde(default = "default_entity_search_limit")]
    pub limit: u32,
}

fn default_memories_per_entity() -> u32 {
    5
}

fn default_entity_search_limit() -> u32 {
    50
}

/// Get entity relations request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetEntityRelationsRequest {
    pub entity_id: String,
    /// Filter by relation type.
    #[serde(default)]
    pub relation_type: Option<EntityRelationType>,
    /// Maximum related entities to return.
    #[serde(default = "default_related_limit")]
    pub limit: u32,
}

fn default_related_limit() -> u32 {
    20
}

/// Batch hybrid search request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchHybridSearchRequest {
    pub user_id: String,
    #[serde(default)]
    pub readable_cube_ids: Option<Vec<String>>,
    /// List of queries.
    pub queries: Vec<String>,
    #[serde(default = "default_top_k")]
    pub top_k: u32,
    #[serde(default)]
    pub mode: HybridSearchMode,
    #[serde(default)]
    pub fusion_weights: Option<FusionWeights>,
}

/// Response for batch hybrid search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchHybridSearchResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<BatchHybridSearchData>,
}

/// Batch search result data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchHybridSearchData {
    /// Results per query.
    pub results: Vec<HybridSearchData>,
    /// Total processing time.
    pub total_latency_ms: u64,
}

// ============================================================================
// Session Management DTOs (P1-3)
// ============================================================================

/// Create a new session request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub user_id: String,
    /// Optional session title
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Session response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    pub session_id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub memory_count: u64,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// List sessions request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSessionsRequest {
    pub user_id: String,
    #[serde(default = "default_session_limit")]
    pub limit: u32,
    #[serde(default)]
    pub cursor: Option<String>,
}

fn default_session_limit() -> u32 {
    50
}

/// Session list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSessionsResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<ListSessionsData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSessionsData {
    pub sessions: Vec<SessionResponse>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Delete session request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteSessionRequest {
    pub session_id: String,
    pub user_id: String,
    /// Whether to delete all memories in the session
    #[serde(default)]
    pub delete_memories: bool,
}

/// Session timeline request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTimelineRequest {
    pub session_id: String,
    pub user_id: String,
    #[serde(default = "default_timeline_limit")]
    pub limit: u32,
    #[serde(default)]
    pub include_metadata: bool,
}

fn default_timeline_limit() -> u32 {
    50
}

/// Session timeline response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTimelineResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<SessionTimelineData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTimelineData {
    pub session_id: String,
    pub memories: Vec<MemoryItem>,
    pub total: u32,
}

// ============================================================================
// Batch Operations DTOs (P1-2)
// ============================================================================

/// Batch add memories request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchAddRequest {
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    pub memories: Vec<BatchMemoryContent>,
    /// Processing mode: "parallel" or "sequential"
    #[serde(default = "default_batch_mode")]
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchMemoryContent {
    pub memory: String,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub scope: Option<String>,
}

fn default_batch_mode() -> String {
    "parallel".to_string()
}

/// Batch add response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchAddResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<BatchAddData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchAddData {
    pub successful: Vec<BatchResult>,
    pub failed: Vec<BatchFailure>,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub memory_id: String,
    pub index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchFailure {
    pub index: u32,
    pub error: String,
}

/// Batch delete request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDeleteRequest {
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    pub memory_ids: Vec<String>,
    #[serde(default)]
    pub soft: bool,
}

/// Batch delete response.
pub type BatchDeleteResponse = BatchAddResponse;

/// Export request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRequest {
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    /// Scope to export: all, WorkingMemory, UserMemory, LongTermMemory
    #[serde(default = "default_export_scope")]
    pub scope: String,
    /// Export format: json, jsonl
    #[serde(default = "default_export_format")]
    pub format: String,
}

fn default_export_scope() -> String {
    "all".to_string()
}
fn default_export_format() -> String {
    "json".to_string()
}

/// Export response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<ExportData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportData {
    pub total_memories: u32,
    pub data: String,
}

// ============================================================================
// Memory Summary DTOs (P1-1)
// ============================================================================

/// Summarize memories request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizeRequest {
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    /// Memory IDs to summarize, or use session_id
    #[serde(default)]
    pub memory_ids: Option<Vec<String>>,
    /// Session ID to summarize all memories in a session
    #[serde(default)]
    pub session_id: Option<String>,
    /// Max words in summary
    #[serde(default = "default_summary_max_words")]
    pub max_words: u32,
}

fn default_summary_max_words() -> u32 {
    200
}

/// Summarize response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizeResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<SummarizeData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizeData {
    pub summary: String,
    pub summary_memory_id: String,
    pub summarized_count: u32,
}
