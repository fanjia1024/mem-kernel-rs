//! Traits for MemCube and storage backends.

use crate::{ApiAddRequest, ApiSearchRequest, MemoryNode, MemoryResponse, SearchResponse};
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
}

/// Item for vector store (id, vector, payload).
#[derive(Clone, Debug)]
pub struct VecStoreItem {
    pub id: String,
    pub vector: Vec<f32>,
    pub payload: HashMap<String, serde_json::Value>,
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

/// MemCube abstraction: add and search memories.
#[async_trait]
pub trait MemCube: Send + Sync {
    /// Add memories from request; returns MemoryResponse.
    async fn add_memories(&self, req: &ApiAddRequest) -> Result<MemoryResponse, MemCubeError>;

    /// Search memories from request; returns SearchResponse.
    async fn search_memories(&self, req: &ApiSearchRequest) -> Result<SearchResponse, MemCubeError>;
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

#[derive(Debug, thiserror::Error)]
pub enum MemCubeError {
    #[error("mem cube error: {0}")]
    Other(String),
    #[error("embedder: {0}")]
    Embedder(#[from] EmbedderError),
    #[error("graph: {0}")]
    Graph(#[from] GraphStoreError),
    #[error("vector: {0}")]
    Vec(#[from] VecStoreError),
}
