//! In-memory graph store with KNN search over embeddings.

use mem_types::{GraphStore, GraphStoreError, MemoryNode, VecSearchHit};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

type ScopeIndex = HashMap<String, HashMap<String, Vec<String>>>;

/// In-memory implementation of GraphStore.
/// Nodes are keyed by id (globally unique); user/scope indexed for get_all_memory_items and search filtering.
pub struct InMemoryGraphStore {
    /// node_id -> node (embedding optional; used for search_by_embedding when present).
    nodes: Arc<RwLock<HashMap<String, MemoryNode>>>,
    /// user_name -> scope -> node_ids (for get_all_memory_items).
    scope_index: Arc<RwLock<ScopeIndex>>,
}

impl InMemoryGraphStore {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            scope_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn scope_for_node(metadata: &HashMap<String, serde_json::Value>) -> String {
        metadata
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("LongTermMemory")
            .to_string()
    }
}

impl Default for InMemoryGraphStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl GraphStore for InMemoryGraphStore {
    async fn add_node(
        &self,
        id: &str,
        memory: &str,
        metadata: &HashMap<String, serde_json::Value>,
        user_name: Option<&str>,
    ) -> Result<(), GraphStoreError> {
        let un = user_name.unwrap_or("");
        let scope = Self::scope_for_node(metadata);
        let mut meta = metadata.clone();
        meta.insert(
            "user_name".to_string(),
            serde_json::Value::String(un.to_string()),
        );
        let node = MemoryNode {
            id: id.to_string(),
            memory: memory.to_string(),
            metadata: meta,
            embedding: None,
        };
        {
            let mut nodes = self.nodes.write().await;
            nodes.insert(id.to_string(), node);
        }
        {
            let mut idx = self.scope_index.write().await;
            let user_map = idx.entry(un.to_string()).or_default();
            let scope_list = user_map.entry(scope).or_default();
            if !scope_list.contains(&id.to_string()) {
                scope_list.push(id.to_string());
            }
        }
        Ok(())
    }

    async fn add_nodes_batch(
        &self,
        nodes: &[MemoryNode],
        user_name: Option<&str>,
    ) -> Result<(), GraphStoreError> {
        let un = user_name.unwrap_or("");
        let mut guard = self.nodes.write().await;
        let mut idx_guard = self.scope_index.write().await;
        let user_map = idx_guard.entry(un.to_string()).or_default();
        for node in nodes {
            let scope = Self::scope_for_node(&node.metadata);
            let mut n = node.clone();
            n.metadata.insert(
                "user_name".to_string(),
                serde_json::Value::String(un.to_string()),
            );
            guard.insert(n.id.clone(), n);
            let scope_list = user_map.entry(scope).or_default();
            if !scope_list.contains(&node.id) {
                scope_list.push(node.id.clone());
            }
        }
        Ok(())
    }

    async fn get_node(
        &self,
        id: &str,
        include_embedding: bool,
    ) -> Result<Option<MemoryNode>, GraphStoreError> {
        let guard = self.nodes.read().await;
        let mut out = guard.get(id).cloned();
        if let Some(ref mut n) = out {
            if !include_embedding {
                n.embedding = None;
            }
        }
        Ok(out)
    }

    async fn get_nodes(
        &self,
        ids: &[String],
        include_embedding: bool,
    ) -> Result<Vec<MemoryNode>, GraphStoreError> {
        let guard = self.nodes.read().await;
        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(node) = guard.get(id) {
                let mut n = node.clone();
                if !include_embedding {
                    n.embedding = None;
                }
                result.push(n);
            }
        }
        Ok(result)
    }

    async fn search_by_embedding(
        &self,
        vector: &[f32],
        top_k: usize,
        user_name: Option<&str>,
    ) -> Result<Vec<VecSearchHit>, GraphStoreError> {
        let guard = self.nodes.read().await;
        let un = user_name.unwrap_or("");
        let mut candidates: Vec<(String, f64)> = Vec::new();
        for node in guard.values() {
            if !un.is_empty() {
                let node_user = node
                    .metadata
                    .get("user_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if node_user != un {
                    continue;
                }
            }
            let emb = match &node.embedding {
                Some(e) => e,
                None => continue,
            };
            if emb.len() != vector.len() {
                continue;
            }
            let score = cosine_similarity(vector, emb);
            candidates.push((node.id.clone(), score));
        }
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let hits = candidates
            .into_iter()
            .take(top_k)
            .map(|(id, score)| VecSearchHit { id, score })
            .collect();
        Ok(hits)
    }

    async fn get_all_memory_items(
        &self,
        scope: &str,
        user_name: &str,
        include_embedding: bool,
    ) -> Result<Vec<MemoryNode>, GraphStoreError> {
        let ids = {
            let idx = self.scope_index.read().await;
            idx.get(user_name)
                .and_then(|m| m.get(scope))
                .cloned()
                .unwrap_or_default()
        };
        let mut nodes = self.get_nodes(&ids, include_embedding).await?;
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(nodes)
    }

    async fn update_node(
        &self,
        id: &str,
        fields: &HashMap<String, serde_json::Value>,
        user_name: Option<&str>,
    ) -> Result<(), GraphStoreError> {
        let mut guard = self.nodes.write().await;
        let node = guard
            .get_mut(id)
            .ok_or_else(|| GraphStoreError::Other(format!("node not found: {}", id)))?;
        if let Some(un) = user_name {
            let node_owner = node
                .metadata
                .get("user_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if node_owner != un {
                return Err(GraphStoreError::Other(format!(
                    "node not found or access denied: {}",
                    id
                )));
            }
        }
        for (k, v) in fields {
            if k == "memory" {
                node.memory = v.as_str().unwrap_or("").to_string();
            } else {
                node.metadata.insert(k.clone(), v.clone());
            }
        }
        Ok(())
    }

    async fn delete_node(&self, id: &str, user_name: Option<&str>) -> Result<(), GraphStoreError> {
        {
            let nodes = self.nodes.read().await;
            let node = nodes
                .get(id)
                .ok_or_else(|| GraphStoreError::Other(format!("node not found: {}", id)))?;
            if let Some(un) = user_name {
                let node_owner = node
                    .metadata
                    .get("user_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if node_owner != un {
                    return Err(GraphStoreError::Other(format!(
                        "node not found or access denied: {}",
                        id
                    )));
                }
            }
        }
        let mut nodes = self.nodes.write().await;
        nodes
            .remove(id)
            .ok_or_else(|| GraphStoreError::Other(format!("node not found: {}", id)))?;
        let mut idx = self.scope_index.write().await;
        for scope_map in idx.values_mut() {
            for list in scope_map.values_mut() {
                list.retain(|x| x != id);
            }
        }
        Ok(())
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (*x as f64) * (*y as f64))
        .sum();
    let na: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}
