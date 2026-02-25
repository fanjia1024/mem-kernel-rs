//! NaiveMemCube: single MemCube with text_mem path.

use chrono::Utc;
use mem_embed::Embedder;
use mem_graph::GraphStore;
use mem_types::*;
use mem_vec::VecStore;
use std::collections::HashMap;
use uuid::Uuid;

/// MemCube that composes a graph store, vector store, and embedder for add/search.
pub struct NaiveMemCube<G, V, E> {
    pub graph: G,
    pub vec_store: V,
    pub embedder: E,
    /// Default scope for new memories (e.g. LongTermMemory).
    pub default_scope: String,
}

impl<G, V, E> NaiveMemCube<G, V, E>
where
    G: GraphStore + Send + Sync,
    V: VecStore + Send + Sync,
    E: Embedder + Send + Sync,
{
    pub fn new(graph: G, vec_store: V, embedder: E) -> Self {
        Self {
            graph,
            vec_store,
            embedder,
            default_scope: "LongTermMemory".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl<G, V, E> MemCube for NaiveMemCube<G, V, E>
where
    G: GraphStore + Send + Sync,
    V: VecStore + Send + Sync,
    E: Embedder + Send + Sync,
{
    async fn add_memories(&self, req: &ApiAddRequest) -> Result<MemoryResponse, MemCubeError> {
        let content = req.content_to_store().ok_or_else(|| {
            MemCubeError::Other("no messages or memory_content in request".to_string())
        })?;
        let cube_ids = req.writable_cube_ids();
        let user_name = cube_ids.first().map(String::as_str).unwrap_or(&req.user_id);

        let id = Uuid::new_v4().to_string();
        let embedding = self.embedder.embed(&content).await?;
        let mut metadata = HashMap::new();
        metadata.insert(
            "scope".to_string(),
            serde_json::Value::String(self.default_scope.clone()),
        );
        metadata.insert(
            "created_at".to_string(),
            serde_json::Value::String(Utc::now().to_rfc3339()),
        );

        let node = MemoryNode {
            id: id.clone(),
            memory: content.clone(),
            metadata: metadata.clone(),
            embedding: Some(embedding.clone()),
        };
        self.graph
            .add_nodes_batch(&[node], Some(user_name))
            .await
            .map_err(MemCubeError::Graph)?;

        let payload = {
            let mut p = HashMap::new();
            p.insert(
                "mem_cube_id".to_string(),
                serde_json::Value::String(user_name.to_string()),
            );
            p.insert(
                "memory_type".to_string(),
                serde_json::Value::String("text_mem".to_string()),
            );
            p
        };
        let item = VecStoreItem {
            id: id.clone(),
            vector: embedding,
            payload,
        };
        self.vec_store
            .add(&[item], None)
            .await
            .map_err(MemCubeError::Vec)?;

        let data = vec![serde_json::json!({ "id": id, "memory": content })];
        Ok(MemoryResponse {
            code: 200,
            message: "Memory added successfully".to_string(),
            data: Some(data),
        })
    }

    async fn search_memories(
        &self,
        req: &ApiSearchRequest,
    ) -> Result<SearchResponse, MemCubeError> {
        let cube_ids = req.readable_cube_ids();
        let user_name = cube_ids.first().map(String::as_str).unwrap_or(&req.user_id);

        let query_vector = self.embedder.embed(&req.query).await?;
        let top_k = req.top_k as usize;

        let mut filter = req.filter.clone().unwrap_or_default();
        // Always enforce cube boundary even if caller passes conflicting filter.
        filter.insert(
            "mem_cube_id".to_string(),
            serde_json::Value::String(user_name.to_string()),
        );

        let mut hits = self
            .vec_store
            .search(&query_vector, top_k, Some(&filter), None)
            .await
            .map_err(MemCubeError::Vec)?;
        if req.relativity > 0.0 {
            hits.retain(|h| h.score >= req.relativity);
        }

        let ids: Vec<String> = hits.iter().map(|h| h.id.clone()).collect();
        if ids.is_empty() {
            return Ok(SearchResponse {
                code: 200,
                message: "Search completed successfully".to_string(),
                data: Some(SearchResponseData {
                    text_mem: vec![MemoryBucket {
                        memories: vec![],
                        total_nodes: Some(0),
                    }],
                    pref_mem: vec![],
                }),
            });
        }

        let nodes = self
            .graph
            .get_nodes(&ids, false)
            .await
            .map_err(MemCubeError::Graph)?;

        let memories: Vec<MemoryItem> = nodes
            .into_iter()
            .filter(|n| {
                n.metadata
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("active")
                    != "tombstone"
            })
            .map(|n| {
                let mut meta = n.metadata.clone();
                if let Some(score) = hits.iter().find(|h| h.id == n.id).map(|h| h.score) {
                    meta.insert(
                        "relativity".to_string(),
                        serde_json::Value::Number(
                            serde_json::Number::from_f64(score)
                                .unwrap_or(serde_json::Number::from(0)),
                        ),
                    );
                }
                MemoryItem {
                    id: n.id,
                    memory: n.memory,
                    metadata: meta,
                }
            })
            .collect();

        let bucket = MemoryBucket {
            total_nodes: Some(memories.len()),
            memories,
        };
        Ok(SearchResponse {
            code: 200,
            message: "Search completed successfully".to_string(),
            data: Some(SearchResponseData {
                text_mem: vec![bucket],
                pref_mem: vec![],
            }),
        })
    }

    async fn update_memory(
        &self,
        req: &UpdateMemoryRequest,
    ) -> Result<UpdateMemoryResponse, MemCubeError> {
        let user_name = req.mem_cube_id.as_deref().unwrap_or(req.user_id.as_str());
        let id = &req.memory_id;

        let existing = self
            .graph
            .get_node(id, false)
            .await
            .map_err(MemCubeError::Graph)?;
        let node =
            existing.ok_or_else(|| MemCubeError::NotFound(format!("memory not found: {}", id)))?;
        let node_owner = node
            .metadata
            .get("user_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if node_owner != user_name {
            return Err(MemCubeError::NotFound(format!("memory not found: {}", id)));
        }

        let mut fields = HashMap::new();
        if let Some(ref memory) = req.memory {
            fields.insert(
                "memory".to_string(),
                serde_json::Value::String(memory.clone()),
            );
        }
        if let Some(ref meta) = req.metadata {
            for (k, v) in meta {
                fields.insert(k.clone(), v.clone());
            }
        }
        fields.insert(
            "updated_at".to_string(),
            serde_json::Value::String(Utc::now().to_rfc3339()),
        );

        if fields.len() > 1 || req.memory.is_some() {
            self.graph
                .update_node(id, &fields, Some(user_name))
                .await
                .map_err(MemCubeError::Graph)?;
        }

        if let Some(ref new_memory) = req.memory {
            let embedding = self.embedder.embed(new_memory).await?;
            let payload = {
                let mut p = HashMap::new();
                p.insert(
                    "mem_cube_id".to_string(),
                    serde_json::Value::String(user_name.to_string()),
                );
                p.insert(
                    "memory_type".to_string(),
                    serde_json::Value::String("text_mem".to_string()),
                );
                p
            };
            let item = VecStoreItem {
                id: id.to_string(),
                vector: embedding,
                payload,
            };
            self.vec_store
                .upsert(&[item], None)
                .await
                .map_err(MemCubeError::Vec)?;
        }

        let data = vec![serde_json::json!({ "id": id, "updated": true })];
        Ok(UpdateMemoryResponse {
            code: 200,
            message: "Memory updated successfully".to_string(),
            data: Some(data),
        })
    }

    async fn forget_memory(
        &self,
        req: &ForgetMemoryRequest,
    ) -> Result<ForgetMemoryResponse, MemCubeError> {
        let id = &req.memory_id;
        let user_name = req.mem_cube_id.as_deref().unwrap_or(req.user_id.as_str());

        let existing = self
            .graph
            .get_node(id, false)
            .await
            .map_err(MemCubeError::Graph)?;
        let node =
            existing.ok_or_else(|| MemCubeError::NotFound(format!("memory not found: {}", id)))?;
        let node_owner = node
            .metadata
            .get("user_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if node_owner != user_name {
            return Err(MemCubeError::NotFound(format!("memory not found: {}", id)));
        }

        if req.soft {
            let mut fields = HashMap::new();
            fields.insert(
                "state".to_string(),
                serde_json::Value::String("tombstone".to_string()),
            );
            fields.insert(
                "updated_at".to_string(),
                serde_json::Value::String(Utc::now().to_rfc3339()),
            );
            self.graph
                .update_node(id, &fields, Some(user_name))
                .await
                .map_err(MemCubeError::Graph)?;
            self.vec_store
                .delete(&[id.to_string()], None)
                .await
                .map_err(MemCubeError::Vec)?;
        } else {
            self.graph
                .delete_node(id, Some(user_name))
                .await
                .map_err(MemCubeError::Graph)?;
            self.vec_store
                .delete(&[id.to_string()], None)
                .await
                .map_err(MemCubeError::Vec)?;
        }
        let data = vec![serde_json::json!({ "id": id, "forgotten": true })];
        Ok(ForgetMemoryResponse {
            code: 200,
            message: "Memory forgotten successfully".to_string(),
            data: Some(data),
        })
    }

    async fn get_memory(&self, req: &GetMemoryRequest) -> Result<GetMemoryResponse, MemCubeError> {
        let user_name = req.mem_cube_id.as_deref().unwrap_or(req.user_id.as_str());
        let node_opt = self
            .graph
            .get_node(&req.memory_id, false)
            .await
            .map_err(MemCubeError::Graph)?;
        let node = match node_opt {
            Some(n) => n,
            None => {
                return Ok(GetMemoryResponse {
                    code: 404,
                    message: "Memory not found".to_string(),
                    data: None,
                });
            }
        };
        let node_user = node
            .metadata
            .get("user_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if node_user != user_name {
            return Ok(GetMemoryResponse {
                code: 404,
                message: "Memory not found".to_string(),
                data: None,
            });
        }
        let state = node
            .metadata
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("active");
        if state == "tombstone" && !req.include_deleted {
            return Ok(GetMemoryResponse {
                code: 404,
                message: "Memory not found".to_string(),
                data: None,
            });
        }
        let item = MemoryItem {
            id: node.id,
            memory: node.memory,
            metadata: node.metadata,
        };
        Ok(GetMemoryResponse {
            code: 200,
            message: "Success".to_string(),
            data: Some(item),
        })
    }
}
