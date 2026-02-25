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

    fn node_owner(metadata: &HashMap<String, serde_json::Value>) -> &str {
        metadata
            .get("user_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }

    fn parse_cursor(cursor: Option<&str>) -> Result<usize, MemCubeError> {
        match cursor {
            Some(c) => c
                .parse::<usize>()
                .map_err(|_| MemCubeError::BadRequest("invalid graph cursor".to_string())),
            None => Ok(0),
        }
    }

    fn normalize_scope(scope: &str) -> Option<&'static str> {
        let normalized = scope
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-', '_'], "");
        match normalized.as_str() {
            "workingmemory" | "working" | "shortterm" | "shorttermmemory" | "stm" | "recent" => {
                Some(MemoryScope::WorkingMemory.as_str())
            }
            "usermemory" | "user" | "midterm" | "midtermmemory" | "profile" | "preference" => {
                Some(MemoryScope::UserMemory.as_str())
            }
            "longtermmemory" | "longterm" | "ltm" => Some(MemoryScope::LongTermMemory.as_str()),
            _ => None,
        }
    }

    /// Resolves scope from request info, or returns default. Returns `BadRequest` if a scope
    /// is explicitly provided in info but is invalid or not a string (consistent with `update_memory`).
    fn resolve_scope_or_error(req: &ApiAddRequest, default_scope: &str) -> Result<String, MemCubeError> {
        let info = req.info.as_ref();
        let scope_value = info.and_then(|i| i.get("scope").or_else(|| i.get("memory_scope")));
        match scope_value {
            None => Ok(default_scope.to_string()),
            Some(v) => match v.as_str() {
                None => Err(MemCubeError::BadRequest(
                    "scope must be a string".to_string(),
                )),
                Some(s) => Self::normalize_scope(s)
                    .map(str::to_string)
                    .ok_or_else(|| {
                        MemCubeError::BadRequest(format!("invalid scope value: {}", s))
                    }),
            },
        }
    }

    fn bucket_name_for_scope(scope: &str) -> Option<&'static str> {
        match scope {
            "WorkingMemory" => Some("short_term"),
            "UserMemory" => Some("mid_term"),
            "LongTermMemory" => Some("long_term"),
            _ => None,
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
        let scope = Self::resolve_scope_or_error(req, &self.default_scope)?;

        let id = Uuid::new_v4().to_string();
        let embedding = self.embedder.embed(&content).await?;
        let mut metadata = HashMap::new();
        metadata.insert(
            "scope".to_string(),
            serde_json::Value::String(scope.clone()),
        );
        metadata.insert(
            "created_at".to_string(),
            serde_json::Value::String(Utc::now().to_rfc3339()),
        );
        if let Some(ref session_id) = req.session_id {
            metadata.insert(
                "session_id".to_string(),
                serde_json::Value::String(session_id.clone()),
            );
        }
        if let Some(ref task_id) = req.task_id {
            metadata.insert(
                "task_id".to_string(),
                serde_json::Value::String(task_id.clone()),
            );
        }
        if let Some(ref custom_tags) = req.custom_tags {
            metadata.insert("custom_tags".to_string(), serde_json::json!(custom_tags));
        }
        if let Some(ref chat_history) = req.chat_history {
            metadata.insert("chat_history".to_string(), serde_json::json!(chat_history));
        }
        if let Some(ref info) = req.info {
            for (k, v) in info {
                metadata.insert(k.clone(), v.clone());
            }
            metadata.insert(
                "scope".to_string(),
                serde_json::Value::String(scope.clone()),
            );
        }

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

        if let Some(relations) = req.relations.as_ref() {
            if !relations.is_empty() {
                let mut edges = Vec::new();
                for rel in relations {
                    let mut base_metadata = rel.metadata.clone();
                    base_metadata.insert(
                        "created_at".to_string(),
                        serde_json::Value::String(Utc::now().to_rfc3339()),
                    );
                    match rel.direction {
                        GraphDirection::Outbound => {
                            edges.push(MemoryEdge {
                                id: Uuid::new_v4().to_string(),
                                from: id.clone(),
                                to: rel.memory_id.clone(),
                                relation: rel.relation.clone(),
                                metadata: base_metadata.clone(),
                            });
                        }
                        GraphDirection::Inbound => {
                            edges.push(MemoryEdge {
                                id: Uuid::new_v4().to_string(),
                                from: rel.memory_id.clone(),
                                to: id.clone(),
                                relation: rel.relation.clone(),
                                metadata: base_metadata.clone(),
                            });
                        }
                        GraphDirection::Both => {
                            edges.push(MemoryEdge {
                                id: Uuid::new_v4().to_string(),
                                from: id.clone(),
                                to: rel.memory_id.clone(),
                                relation: rel.relation.clone(),
                                metadata: base_metadata.clone(),
                            });
                            edges.push(MemoryEdge {
                                id: Uuid::new_v4().to_string(),
                                from: rel.memory_id.clone(),
                                to: id.clone(),
                                relation: rel.relation.clone(),
                                metadata: base_metadata.clone(),
                            });
                        }
                    }
                }
                if let Err(e) = self.graph.add_edges_batch(&edges, Some(user_name)).await {
                    // Keep add operation atomic-ish for graph writes.
                    let _ = self.graph.delete_node(&id, Some(user_name)).await;
                    return Err(MemCubeError::Graph(e));
                }
            }
        }

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
            p.insert("scope".to_string(), serde_json::Value::String(scope));
            p
        };
        let item = VecStoreItem {
            id: id.clone(),
            vector: embedding,
            payload,
        };
        if let Err(e) = self.vec_store.add(&[item], None).await {
            // Avoid partial success: if vec write fails, rollback graph node and edges.
            let _ = self.graph.delete_node(&id, Some(user_name)).await;
            return Err(MemCubeError::Vec(e));
        }

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
                        name: Some("all".to_string()),
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

        let all_bucket = MemoryBucket {
            name: Some("all".to_string()),
            total_nodes: Some(memories.len()),
            memories: memories.clone(),
        };
        let mut text_mem = vec![all_bucket];
        for scope in [
            MemoryScope::WorkingMemory.as_str(),
            MemoryScope::UserMemory.as_str(),
            MemoryScope::LongTermMemory.as_str(),
        ] {
            let scoped: Vec<MemoryItem> = memories
                .iter()
                .filter(|m| {
                    m.metadata
                        .get("scope")
                        .and_then(|v| v.as_str())
                        .map(|s| s == scope)
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            if scoped.is_empty() {
                continue;
            }
            text_mem.push(MemoryBucket {
                name: Self::bucket_name_for_scope(scope).map(str::to_string),
                total_nodes: Some(scoped.len()),
                memories: scoped,
            });
        }
        Ok(SearchResponse {
            code: 200,
            message: "Search completed successfully".to_string(),
            data: Some(SearchResponseData {
                text_mem,
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
        let node_owner = Self::node_owner(&node.metadata);
        if node_owner != user_name {
            return Err(MemCubeError::NotFound(format!("memory not found: {}", id)));
        }
        let mut payload_scope = node
            .metadata
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or(MemoryScope::LongTermMemory.as_str())
            .to_string();

        let mut fields = HashMap::new();
        if let Some(ref memory) = req.memory {
            fields.insert(
                "memory".to_string(),
                serde_json::Value::String(memory.clone()),
            );
        }
        let mut scope_changed = false;
        if let Some(ref meta) = req.metadata {
            for (k, v) in meta {
                if k == "scope" {
                    if let Some(raw_scope) = v.as_str() {
                        if let Some(normalized_scope) = Self::normalize_scope(raw_scope) {
                            payload_scope = normalized_scope.to_string();
                            fields.insert(
                                "scope".to_string(),
                                serde_json::Value::String(payload_scope.clone()),
                            );
                            scope_changed = true;
                        } else {
                            return Err(MemCubeError::BadRequest(format!(
                                "invalid scope value: {}",
                                raw_scope
                            )));
                        }
                    } else {
                        return Err(MemCubeError::BadRequest(
                            "scope must be a string".to_string(),
                        ));
                    }
                } else {
                    fields.insert(k.clone(), v.clone());
                }
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

        if req.memory.is_some() || scope_changed {
            let embedding = if let Some(ref new_memory) = req.memory {
                self.embedder.embed(new_memory).await?
            } else {
                let ids = vec![id.to_string()];
                let mut existing_items = self
                    .vec_store
                    .get_by_ids(&ids, None)
                    .await
                    .map_err(MemCubeError::Vec)?;
                if let Some(existing_item) = existing_items.pop() {
                    existing_item.vector
                } else {
                    self.embedder.embed(&node.memory).await?
                }
            };
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
                p.insert(
                    "scope".to_string(),
                    serde_json::Value::String(payload_scope),
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
        let node_owner = Self::node_owner(&node.metadata);
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
        let node_user = Self::node_owner(&node.metadata);
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

    async fn graph_neighbors(
        &self,
        req: &GraphNeighborsRequest,
    ) -> Result<GraphNeighborsResponse, MemCubeError> {
        let user_name = req.mem_cube_id.as_deref().unwrap_or(req.user_id.as_str());
        let offset = Self::parse_cursor(req.cursor.as_deref())?;
        let source = self
            .graph
            .get_node(&req.memory_id, false)
            .await
            .map_err(MemCubeError::Graph)?
            .ok_or_else(|| {
                MemCubeError::NotFound(format!("memory not found: {}", req.memory_id))
            })?;
        if Self::node_owner(&source.metadata) != user_name {
            return Err(MemCubeError::NotFound(format!(
                "memory not found: {}",
                req.memory_id
            )));
        }
        let source_state = source
            .metadata
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("active");
        if source_state == "tombstone" && !req.include_deleted {
            return Err(MemCubeError::NotFound(format!(
                "memory not found: {}",
                req.memory_id
            )));
        }

        let neighbors = self
            .graph
            .get_neighbors(
                &req.memory_id,
                req.relation.as_deref(),
                req.direction,
                usize::MAX,
                req.include_embedding,
                Some(user_name),
            )
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("access denied") {
                    MemCubeError::NotFound(format!("memory not found: {}", req.memory_id))
                } else {
                    MemCubeError::Graph(e)
                }
            })?;

        let all_items: Vec<GraphNeighborItem> = neighbors
            .into_iter()
            .filter(|n| {
                if req.include_deleted {
                    return true;
                }
                n.node
                    .metadata
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("active")
                    != "tombstone"
            })
            .map(|n| GraphNeighborItem {
                edge: n.edge,
                memory: MemoryItem {
                    id: n.node.id,
                    memory: n.node.memory,
                    metadata: n.node.metadata,
                },
            })
            .collect();

        let limit = req.limit as usize;
        let items: Vec<GraphNeighborItem> =
            all_items.iter().skip(offset).take(limit).cloned().collect();
        let next_cursor = if offset + items.len() < all_items.len() {
            Some((offset + items.len()).to_string())
        } else {
            None
        };

        Ok(GraphNeighborsResponse {
            code: 200,
            message: "Success".to_string(),
            data: Some(GraphNeighborsData { items, next_cursor }),
        })
    }

    async fn graph_path(&self, req: &GraphPathRequest) -> Result<GraphPathResponse, MemCubeError> {
        let user_name = req.mem_cube_id.as_deref().unwrap_or(req.user_id.as_str());
        let path = self
            .graph
            .shortest_path(
                &req.source_memory_id,
                &req.target_memory_id,
                req.relation.as_deref(),
                req.direction,
                req.max_depth as usize,
                req.include_deleted,
                Some(user_name),
            )
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("access denied") {
                    MemCubeError::NotFound(format!(
                        "memory not found: {} or {}",
                        req.source_memory_id, req.target_memory_id
                    ))
                } else {
                    MemCubeError::Graph(e)
                }
            })?
            .ok_or_else(|| {
                MemCubeError::NotFound(format!(
                    "path not found: {} -> {}",
                    req.source_memory_id, req.target_memory_id
                ))
            })?;

        let nodes = self
            .graph
            .get_nodes(&path.node_ids, false)
            .await
            .map_err(MemCubeError::Graph)?;
        let items: Vec<MemoryItem> = nodes
            .into_iter()
            .map(|n| MemoryItem {
                id: n.id,
                memory: n.memory,
                metadata: n.metadata,
            })
            .collect();

        Ok(GraphPathResponse {
            code: 200,
            message: "Success".to_string(),
            data: Some(GraphPathData {
                hops: path.edges.len() as u32,
                nodes: items,
                edges: path.edges,
            }),
        })
    }

    async fn graph_paths(
        &self,
        req: &GraphPathsRequest,
    ) -> Result<GraphPathsResponse, MemCubeError> {
        if req.top_k_paths == 0 {
            return Err(MemCubeError::BadRequest(
                "top_k_paths must be greater than 0".to_string(),
            ));
        }
        let user_name = req.mem_cube_id.as_deref().unwrap_or(req.user_id.as_str());
        let paths = self
            .graph
            .find_paths(
                &req.source_memory_id,
                &req.target_memory_id,
                req.relation.as_deref(),
                req.direction,
                req.max_depth as usize,
                req.top_k_paths as usize,
                req.include_deleted,
                Some(user_name),
            )
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("access denied") {
                    MemCubeError::NotFound(format!(
                        "memory not found: {} or {}",
                        req.source_memory_id, req.target_memory_id
                    ))
                } else {
                    MemCubeError::Graph(e)
                }
            })?;
        if paths.is_empty() {
            return Err(MemCubeError::NotFound(format!(
                "path not found: {} -> {}",
                req.source_memory_id, req.target_memory_id
            )));
        }

        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            let nodes = self
                .graph
                .get_nodes(&path.node_ids, false)
                .await
                .map_err(MemCubeError::Graph)?;
            let items: Vec<MemoryItem> = nodes
                .into_iter()
                .map(|n| MemoryItem {
                    id: n.id,
                    memory: n.memory,
                    metadata: n.metadata,
                })
                .collect();
            out.push(GraphPathData {
                hops: path.edges.len() as u32,
                nodes: items,
                edges: path.edges,
            });
        }

        Ok(GraphPathsResponse {
            code: 200,
            message: "Success".to_string(),
            data: Some(out),
        })
    }
}
