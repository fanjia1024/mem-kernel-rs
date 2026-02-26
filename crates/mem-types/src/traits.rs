//! Traits for MemCube and storage backends.

use crate::{
    ApiAddRequest, ApiHybridSearchRequest, ApiSearchRequest, AuditEvent, AuditListOptions,
    ForgetMemoryRequest, ForgetMemoryResponse, GetMemoryRequest, GetMemoryResponse, GraphDirection,
    GraphNeighbor, GraphNeighborsRequest, GraphNeighborsResponse, GraphPath, GraphPathRequest,
    GraphPathResponse, GraphPathsRequest, GraphPathsResponse, HybridSearchResponse, MemoryEdge,
    MemoryNode, MemoryResponse, SearchResponse, UpdateMemoryRequest, UpdateMemoryResponse,
};
use async_trait::async_trait;
use std::collections::HashMap;

/// Result of a vector search hit (id + score).
#[derive(Debug, Clone)]
pub struct VecSearchHit {
    pub id: String,
    pub score: f64,
}

/// Graph store abstraction (subset of MemOS BaseGraphDB).
#[async_trait]
pub trait GraphStore: Send + Sync {
    /// Add a single memory node.
    async fn add_node(
        &self,
        id: &str,
        memory: &str,
        metadata: &HashMap<String, serde_json::Value>,
        user_name: Option<&str>,
    ) -> Result<(), GraphStoreError>;

    /// Add multiple nodes in batch.
    async fn add_nodes_batch(
        &self,
        nodes: &[MemoryNode],
        user_name: Option<&str>,
    ) -> Result<(), GraphStoreError>;

    /// Add multiple edges in batch.
    async fn add_edges_batch(
        &self,
        edges: &[MemoryEdge],
        user_name: Option<&str>,
    ) -> Result<(), GraphStoreError>;

    /// Get one node by id.
    async fn get_node(
        &self,
        id: &str,
        include_embedding: bool,
    ) -> Result<Option<MemoryNode>, GraphStoreError>;

    /// Get multiple nodes by ids.
    async fn get_nodes(
        &self,
        ids: &[String],
        include_embedding: bool,
    ) -> Result<Vec<MemoryNode>, GraphStoreError>;

    /// Get neighbors of one node, optionally filtered by relation and direction.
    async fn get_neighbors(
        &self,
        id: &str,
        relation: Option<&str>,
        direction: GraphDirection,
        limit: usize,
        include_embedding: bool,
        user_name: Option<&str>,
    ) -> Result<Vec<GraphNeighbor>, GraphStoreError>;

    /// Shortest path query between source and target by BFS hops.
    async fn shortest_path(
        &self,
        source_id: &str,
        target_id: &str,
        relation: Option<&str>,
        direction: GraphDirection,
        max_depth: usize,
        include_deleted: bool,
        user_name: Option<&str>,
    ) -> Result<Option<GraphPath>, GraphStoreError>;

    /// Enumerate top-k shortest simple paths by BFS hops.
    async fn find_paths(
        &self,
        source_id: &str,
        target_id: &str,
        relation: Option<&str>,
        direction: GraphDirection,
        max_depth: usize,
        top_k: usize,
        include_deleted: bool,
        user_name: Option<&str>,
    ) -> Result<Vec<GraphPath>, GraphStoreError>;

    /// Search by embedding vector (returns node ids + scores).
    async fn search_by_embedding(
        &self,
        vector: &[f32],
        top_k: usize,
        user_name: Option<&str>,
    ) -> Result<Vec<VecSearchHit>, GraphStoreError>;

    /// Get all memory items for a scope and user.
    async fn get_all_memory_items(
        &self,
        scope: &str,
        user_name: &str,
        include_embedding: bool,
    ) -> Result<Vec<MemoryNode>, GraphStoreError>;

    /// Update fields of an existing node (memory and/or metadata).
    async fn update_node(
        &self,
        id: &str,
        fields: &HashMap<String, serde_json::Value>,
        user_name: Option<&str>,
    ) -> Result<(), GraphStoreError>;

    /// Delete a node (hard delete). If `user_name` is `Some`, implementation must verify
    /// the node belongs to that user/cube (e.g. via metadata) before deleting; return error if not owner.
    async fn delete_node(&self, id: &str, user_name: Option<&str>) -> Result<(), GraphStoreError>;

    /// Delete all edges connected to a node. Returns number of deleted edges.
    async fn delete_edges_by_node(
        &self,
        id: &str,
        user_name: Option<&str>,
    ) -> Result<usize, GraphStoreError>;
}

/// Vector store abstraction (subset of MemOS BaseVecDB).
#[async_trait]
pub trait VecStore: Send + Sync {
    /// Add items (id, vector, payload).
    async fn add(
        &self,
        items: &[VecStoreItem],
        collection: Option<&str>,
    ) -> Result<(), VecStoreError>;

    /// Search by vector.
    async fn search(
        &self,
        query_vector: &[f32],
        top_k: usize,
        filter: Option<&HashMap<String, serde_json::Value>>,
        collection: Option<&str>,
    ) -> Result<Vec<VecSearchHit>, VecStoreError>;

    /// Get by ids.
    async fn get_by_ids(
        &self,
        ids: &[String],
        collection: Option<&str>,
    ) -> Result<Vec<VecStoreItem>, VecStoreError>;

    /// Delete by ids.
    async fn delete(&self, ids: &[String], collection: Option<&str>) -> Result<(), VecStoreError>;

    /// Upsert items: insert or replace by id. Ensures full payload and avoids delete+add window.
    async fn upsert(
        &self,
        items: &[VecStoreItem],
        collection: Option<&str>,
    ) -> Result<(), VecStoreError>;
}

/// Item for vector store (id, vector, payload).
#[derive(Clone, Debug)]
pub struct VecStoreItem {
    pub id: String,
    pub vector: Vec<f32>,
    pub payload: HashMap<String, serde_json::Value>,
}

/// Result of a keyword/BM25 search hit.
#[derive(Debug, Clone)]
pub struct KeywordSearchHit {
    pub id: String,
    pub score: f64,
}

/// Keyword store abstraction for BM25 search.
#[async_trait]
pub trait KeywordStore: Send + Sync {
    /// Index a document (memory) by id and text for the given user/cube.
    async fn index(
        &self,
        memory_id: &str,
        text: &str,
        user_name: Option<&str>,
    ) -> Result<(), KeywordStoreError>;

    /// Remove a document from the index.
    async fn remove(
        &self,
        memory_id: &str,
        user_name: Option<&str>,
    ) -> Result<(), KeywordStoreError>;

    /// Search by query string; returns top-k (id, score) for the user/cube.
    async fn search(
        &self,
        query: &str,
        top_k: usize,
        user_name: Option<&str>,
        filter: Option<&HashMap<String, serde_json::Value>>,
    ) -> Result<Vec<KeywordSearchHit>, KeywordStoreError>;
}

#[derive(Debug, thiserror::Error)]
pub enum KeywordStoreError {
    #[error("keyword store error: {0}")]
    Other(String),
}

/// Single result from a reranker (memory id and relevance score).
#[derive(Debug, Clone)]
pub struct RerankHit {
    pub memory_id: String,
    pub score: f64,
}

/// Reranker: reorder and score a list of documents by relevance to the query.
#[async_trait::async_trait]
pub trait Reranker: Send + Sync {
    /// Rerank documents; returns top_k hits in order (memory_id, score).
    async fn rerank(
        &self,
        query: &str,
        doc_ids: &[String],
        documents: &[String],
        top_k: u32,
    ) -> Result<Vec<RerankHit>, RerankError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RerankError {
    #[error("rerank error: {0}")]
    Other(String),
}

/// Embedder: text -> vector(s).
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embed a single text. Default implementation uses embed_batch.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedderError> {
        let v = self.embed_batch(&[text.to_string()]).await?;
        v.into_iter().next().ok_or(EmbedderError::EmptyResponse)
    }

    /// Embed multiple texts.
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedderError>;
}

/// MemCube abstraction: add, search, update, and forget memories.
#[async_trait]
pub trait MemCube: Send + Sync {
    /// Add memories from request; returns MemoryResponse.
    async fn add_memories(&self, req: &ApiAddRequest) -> Result<MemoryResponse, MemCubeError>;

    /// Search memories from request; returns SearchResponse.
    async fn search_memories(&self, req: &ApiSearchRequest)
        -> Result<SearchResponse, MemCubeError>;

    /// Update an existing memory (partial fields); re-embeds if memory text changed.
    async fn update_memory(
        &self,
        req: &UpdateMemoryRequest,
    ) -> Result<UpdateMemoryResponse, MemCubeError>;

    /// Forget (soft or hard delete) a memory.
    async fn forget_memory(
        &self,
        req: &ForgetMemoryRequest,
    ) -> Result<ForgetMemoryResponse, MemCubeError>;

    /// Get a single memory by id (within user/cube scope).
    async fn get_memory(&self, req: &GetMemoryRequest) -> Result<GetMemoryResponse, MemCubeError>;

    /// Query graph neighbors for one memory id.
    async fn graph_neighbors(
        &self,
        req: &GraphNeighborsRequest,
    ) -> Result<GraphNeighborsResponse, MemCubeError>;

    /// Query shortest path between two memory nodes.
    async fn graph_path(&self, req: &GraphPathRequest) -> Result<GraphPathResponse, MemCubeError>;

    /// Query top-k shortest paths between two memory nodes.
    async fn graph_paths(
        &self,
        req: &GraphPathsRequest,
    ) -> Result<GraphPathsResponse, MemCubeError>;

    /// Hybrid search (vector + optional graph + optional keyword). Default: not supported.
    async fn hybrid_search(
        &self,
        req: &ApiHybridSearchRequest,
    ) -> Result<HybridSearchResponse, MemCubeError> {
        let _ = req;
        Err(MemCubeError::Other(
            "hybrid search not supported".to_string(),
        ))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GraphStoreError {
    #[error("graph store error: {0}")]
    Other(String),
}

#[derive(Debug, thiserror::Error)]
pub enum VecStoreError {
    #[error("vector store error: {0}")]
    Other(String),
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedderError {
    #[error("embedder error: {0}")]
    Other(String),
    #[error("empty response")]
    EmptyResponse,
}

/// Audit event store: append-only log with optional list by user/cube/time and pagination.
#[async_trait]
pub trait AuditStore: Send + Sync {
    /// Append one audit event.
    async fn append(&self, event: AuditEvent) -> Result<(), AuditStoreError>;

    /// List events with optional filters and limit/offset. Newest first.
    async fn list(&self, opts: &AuditListOptions) -> Result<Vec<AuditEvent>, AuditStoreError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AuditStoreError {
    #[error("audit store error: {0}")]
    Other(String),
}

#[derive(Debug, thiserror::Error)]
pub enum MemCubeError {
    #[error("mem cube error: {0}")]
    Other(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("embedder: {0}")]
    Embedder(#[from] EmbedderError),
    #[error("graph: {0}")]
    Graph(#[from] GraphStoreError),
    #[error("vector: {0}")]
    Vec(#[from] VecStoreError),
    #[error("keyword: {0}")]
    Keyword(#[from] KeywordStoreError),
}
