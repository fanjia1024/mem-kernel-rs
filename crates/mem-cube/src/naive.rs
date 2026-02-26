//! NaiveMemCube: single MemCube with text_mem path.

use chrono::Utc;
use mem_embed::{Embedder, LLMClient};
use mem_graph::GraphStore;
use mem_types::*;
use mem_vec::VecStore;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Candidate from merging vector/graph/keyword channel hits (id + per-channel scores).
struct HybridCandidate {
    id: String,
    vector_score: Option<f64>,
    graph_score: Option<f64>,
    keyword_score: Option<f64>,
}

/// MemCube that composes a graph store, vector store, and embedder for add/search.
pub struct NaiveMemCube<G, V, E> {
    pub graph: G,
    pub vec_store: V,
    pub embedder: E,
    /// Default scope for new memories (e.g. LongTermMemory).
    pub default_scope: String,
    /// Optional keyword store for BM25 channel in hybrid search.
    pub keyword_store: Option<Arc<dyn KeywordStore + Send + Sync>>,
    /// Optional reranker for hybrid search.
    pub reranker: Option<Arc<dyn Reranker + Send + Sync>>,
    /// Optional LLM client for memory summarization (P1-1).
    pub llm_client: Option<Arc<dyn LLMClient + Send + Sync>>,
    /// Optional session store for session management (P1-3).
    pub session_store: Option<Arc<dyn SessionStore + Send + Sync>>,
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
            keyword_store: None,
            reranker: None,
            llm_client: None,
            session_store: None,
        }
    }

    /// Attach an optional reranker for hybrid search.
    pub fn with_reranker(mut self, reranker: Option<Arc<dyn Reranker + Send + Sync>>) -> Self {
        self.reranker = reranker;
        self
    }

    /// Attach an optional keyword store for hybrid search BM25 channel.
    pub fn with_keyword_store(
        mut self,
        keyword_store: Option<Arc<dyn KeywordStore + Send + Sync>>,
    ) -> Self {
        self.keyword_store = keyword_store;
        self
    }

    /// Attach an optional LLM client for memory summarization (P1-1).
    pub fn with_llm_client(mut self, llm_client: Option<Arc<dyn LLMClient + Send + Sync>>) -> Self {
        self.llm_client = llm_client;
        self
    }

    /// Attach an optional session store for session management (P1-3).
    pub fn with_session_store(
        mut self,
        session_store: Option<Arc<dyn SessionStore + Send + Sync>>,
    ) -> Self {
        self.session_store = session_store;
        self
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
    fn resolve_scope_or_error(
        req: &ApiAddRequest,
        default_scope: &str,
    ) -> Result<String, MemCubeError> {
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
                    .ok_or_else(|| MemCubeError::BadRequest(format!("invalid scope value: {}", s))),
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

    /// Merge channel hits into deduplicated candidates and build channel_results.
    fn merge_hybrid_candidates(
        vector_hits: &[VecSearchHit],
        graph_hits: &[VecSearchHit],
        keyword_hits: &[(String, f64)],
    ) -> (Vec<HybridCandidate>, Vec<ChannelResult>) {
        use std::collections::HashMap;
        let mut by_id: HashMap<String, HybridCandidate> = HashMap::new();
        for h in vector_hits {
            by_id
                .entry(h.id.clone())
                .or_insert_with(|| HybridCandidate {
                    id: h.id.clone(),
                    vector_score: None,
                    graph_score: None,
                    keyword_score: None,
                })
                .vector_score = Some(h.score);
        }
        for h in graph_hits {
            by_id
                .entry(h.id.clone())
                .or_insert_with(|| HybridCandidate {
                    id: h.id.clone(),
                    vector_score: None,
                    graph_score: None,
                    keyword_score: None,
                })
                .graph_score = Some(h.score);
        }
        for (id, score) in keyword_hits {
            by_id
                .entry(id.clone())
                .or_insert_with(|| HybridCandidate {
                    id: id.clone(),
                    vector_score: None,
                    graph_score: None,
                    keyword_score: None,
                })
                .keyword_score = Some(*score);
        }
        let candidates: Vec<HybridCandidate> = by_id.into_values().collect();
        let channel_results = vec![
            ChannelResult {
                channel: SearchChannel::Vector,
                count: vector_hits.len() as u32,
                hits: vec![],
            },
            ChannelResult {
                channel: SearchChannel::Graph,
                count: graph_hits.len() as u32,
                hits: vec![],
            },
            ChannelResult {
                channel: SearchChannel::Keyword,
                count: keyword_hits.len() as u32,
                hits: vec![],
            },
        ];
        (candidates, channel_results)
    }

    fn channels_for_scores(v: Option<f64>, g: Option<f64>, k: Option<f64>) -> Vec<SearchChannel> {
        let mut ch = vec![];
        if v.is_some() {
            ch.push(SearchChannel::Vector);
        }
        if g.is_some() {
            ch.push(SearchChannel::Graph);
        }
        if k.is_some() {
            ch.push(SearchChannel::Keyword);
        }
        ch
    }

    /// P0: Filter nodes by time range (since/until/time_range)
    fn filter_nodes_by_time(nodes: Vec<MemoryNode>, req: &ApiSearchRequest) -> Vec<MemoryNode> {
        // If no time filters, return all
        if req.since.is_none() && req.until.is_none() && req.time_range.is_none() {
            return nodes;
        }

        nodes
            .into_iter()
            .filter(|n| {
                let created_at = n
                    .metadata
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Handle time_range field
                let (start, end) = if let Some(ref tr) = req.time_range {
                    (Some(tr.start.as_str()), Some(tr.end.as_str()))
                } else {
                    (req.since.as_deref(), req.until.as_deref())
                };

                // Check start time
                let passes_start = start.map(|s| created_at >= s).unwrap_or(true);

                // Check end time
                let passes_end = end.map(|e| created_at <= e).unwrap_or(true);

                passes_start && passes_end
            })
            .collect()
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

        if let Some(ref kw) = self.keyword_store {
            if let Err(e) = kw.index(&id, &content, Some(user_name)).await {
                #[allow(clippy::cloned_ref_to_slice_refs)]
                let _ = self.vec_store.delete(&[id.clone()], None).await;
                let _ = self.graph.delete_node(&id, Some(user_name)).await;
                return Err(MemCubeError::Keyword(e));
            }
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

        // P0: Apply time range filtering
        let nodes = Self::filter_nodes_by_time(nodes, req);

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

    async fn hybrid_search(
        &self,
        req: &ApiHybridSearchRequest,
    ) -> Result<HybridSearchResponse, MemCubeError> {
        let start = std::time::Instant::now();
        let cube_ids = req.readable_cube_ids();
        let user_name = cube_ids.first().map(String::as_str).unwrap_or(&req.user_id);
        let top_k = req.top_k as usize;
        let mut filter = std::collections::HashMap::new();
        filter.insert(
            "mem_cube_id".to_string(),
            serde_json::Value::String(user_name.to_string()),
        );

        let query_vector = self.embedder.embed(&req.query).await?;

        let keyword_enabled = self.keyword_store.is_some()
            && req
                .keyword_config
                .as_ref()
                .map(|c| c.enabled)
                .unwrap_or(true);

        let (vector_hits, graph_hits, keyword_hits): (
            Vec<VecSearchHit>,
            Vec<VecSearchHit>,
            Vec<KeywordSearchHit>,
        ) = match req.mode {
            HybridSearchMode::VectorOnly => {
                let v = self
                    .vec_store
                    .search(&query_vector, top_k, Some(&filter), None)
                    .await
                    .map_err(MemCubeError::Vec)?;
                (v, vec![], vec![])
            }
            HybridSearchMode::GraphOnly => {
                let g = self
                    .graph
                    .search_by_embedding(&query_vector, top_k, Some(user_name))
                    .await
                    .map_err(MemCubeError::Graph)?;
                (vec![], g, vec![])
            }
            HybridSearchMode::KeywordOnly => {
                let kw = self.keyword_store.as_ref().ok_or_else(|| {
                    MemCubeError::Other("keyword-only search requires keyword store".to_string())
                })?;
                let k = kw
                    .search(&req.query, top_k, Some(user_name), Some(&filter))
                    .await?;
                (vec![], vec![], k)
            }
            HybridSearchMode::Fusion | HybridSearchMode::Custom => {
                if keyword_enabled {
                    let kw = self.keyword_store.as_ref().unwrap();
                    let (v_res, g_res, k_res) = tokio::join!(
                        self.vec_store
                            .search(&query_vector, top_k, Some(&filter), None),
                        self.graph
                            .search_by_embedding(&query_vector, top_k, Some(user_name)),
                        kw.search(&req.query, top_k, Some(user_name), Some(&filter)),
                    );
                    let v = v_res.map_err(MemCubeError::Vec)?;
                    let g = g_res.map_err(MemCubeError::Graph)?;
                    let k = k_res?;
                    (v, g, k)
                } else {
                    let (v_res, g_res) = tokio::join!(
                        self.vec_store
                            .search(&query_vector, top_k, Some(&filter), None),
                        self.graph
                            .search_by_embedding(&query_vector, top_k, Some(user_name)),
                    );
                    let v = v_res.map_err(MemCubeError::Vec)?;
                    let g = g_res.map_err(MemCubeError::Graph)?;
                    (v, g, vec![])
                }
            }
        };

        let kw_pairs: Vec<(String, f64)> = keyword_hits
            .iter()
            .map(|h| (h.id.clone(), h.score))
            .collect();
        let (candidates, channel_results) =
            Self::merge_hybrid_candidates(&vector_hits, &graph_hits, &kw_pairs);

        if candidates.is_empty() {
            let latency_ms = start.elapsed().as_millis() as u64;
            return Ok(HybridSearchResponse {
                code: 200,
                message: "Hybrid search completed successfully".to_string(),
                data: Some(HybridSearchData {
                    query: req.query.clone(),
                    total_candidates: 0,
                    hits: vec![],
                    channel_results,
                    rerank_used: false,
                    latency_ms,
                }),
            });
        }

        let ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
        let nodes = self
            .graph
            .get_nodes(&ids, false)
            .await
            .map_err(MemCubeError::Graph)?;

        let score_map: std::collections::HashMap<String, (Option<f64>, Option<f64>, Option<f64>)> =
            candidates
                .iter()
                .map(|c| {
                    (
                        c.id.clone(),
                        (c.vector_score, c.graph_score, c.keyword_score),
                    )
                })
                .collect();

        let max_keyword = candidates
            .iter()
            .filter_map(|c| c.keyword_score)
            .fold(0.0f64, |a, b| a.max(b));
        let keyword_scale = if max_keyword > 0.0 { max_keyword } else { 1.0 };

        let weights = req
            .fusion_weights
            .as_ref()
            .map(|w| (w.vector_weight, w.keyword_weight, w.graph_weight))
            .unwrap_or((0.6, 0.3, 0.1));

        let mut hits: Vec<HybridSearchHit> = nodes
            .into_iter()
            .filter(|n| {
                n.metadata
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("active")
                    != "tombstone"
            })
            .filter_map(|n| {
                let scores = score_map.get(&n.id)?;
                let v_norm = scores.0.unwrap_or(0.0);
                let g_norm = scores.1.unwrap_or(0.0);
                let k_raw = scores.2.unwrap_or(0.0);
                let k_norm = if keyword_scale > 0.0 {
                    k_raw / keyword_scale
                } else {
                    k_raw
                };
                let fused = weights.0 * v_norm + weights.1 * k_norm + weights.2 * g_norm;
                Some(HybridSearchHit {
                    memory_id: n.id.clone(),
                    memory_content: n.memory.clone(),
                    metadata: n.metadata.clone(),
                    vector_score: scores.0,
                    keyword_score: scores.2,
                    graph_score: scores.1,
                    fused_score: fused,
                    vector_norm: scores.0,
                    keyword_norm: scores.2.map(|_| k_norm),
                    graph_norm: scores.1,
                    rerank_score: None,
                    channels: Self::channels_for_scores(scores.0, scores.1, scores.2),
                })
            })
            .collect();

        hits.sort_by(|a, b| {
            b.fused_score
                .partial_cmp(&a.fused_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let total_candidates = hits.len();

        let (hits, rerank_used) = if let (Some(reranker), Some(rcfg)) =
            (self.reranker.as_ref(), req.rerank_config.as_ref())
        {
            if rcfg.enabled && rcfg.model_url.is_some() && !hits.is_empty() {
                let rerank_top_k = rcfg.rerank_top_k as usize;
                let take = (rerank_top_k * 2).min(hits.len());
                let candidates: Vec<_> = hits.iter().take(take).cloned().collect();
                let ids: Vec<String> = candidates.iter().map(|h| h.memory_id.clone()).collect();
                let docs: Vec<String> = candidates
                    .iter()
                    .map(|h| h.memory_content.clone())
                    .collect();
                match reranker
                    .rerank(&req.query, &ids, &docs, rcfg.rerank_top_k)
                    .await
                {
                    Ok(reranked) if !reranked.is_empty() => {
                        let id_to_hit: std::collections::HashMap<_, _> = candidates
                            .into_iter()
                            .map(|h| (h.memory_id.clone(), h))
                            .collect();
                        let mut out: Vec<HybridSearchHit> = reranked
                            .into_iter()
                            .filter_map(|r| {
                                let mut h = id_to_hit.get(&r.memory_id)?.clone();
                                h.rerank_score = Some(r.score);
                                Some(h)
                            })
                            .collect();
                        out.truncate(top_k);
                        (out, true)
                    }
                    _ => {
                        hits.truncate(top_k);
                        (hits, false)
                    }
                }
            } else {
                hits.truncate(top_k);
                (hits, false)
            }
        } else {
            hits.truncate(top_k);
            (hits, false)
        };

        let latency_ms = start.elapsed().as_millis() as u64;
        Ok(HybridSearchResponse {
            code: 200,
            message: "Hybrid search completed successfully".to_string(),
            data: Some(HybridSearchData {
                query: req.query.clone(),
                total_candidates: total_candidates as u32,
                hits,
                channel_results,
                rerank_used,
                latency_ms,
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

        if let Some(ref kw) = self.keyword_store {
            let content = req.memory.as_deref().unwrap_or(&node.memory);
            let _ = kw.index(id, content, Some(user_name)).await;
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
        if let Some(ref kw) = self.keyword_store {
            let _ = kw.remove(id, Some(user_name)).await;
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

    // ============================================================================
    // Batch Operations (P1-2)
    // ============================================================================

    async fn add_memories_batch(
        &self,
        req: &BatchAddRequest,
    ) -> Result<BatchAddResponse, MemCubeError> {
        let cube_ids = req
            .mem_cube_id
            .as_ref()
            .map(|id| vec![id.clone()])
            .unwrap_or_else(|| vec![req.user_id.clone()]);
        let user_name = cube_ids.first().map(String::as_str).unwrap_or(&req.user_id);

        let mut successful = Vec::new();
        let mut failed = Vec::new();

        // Parallel embedding generation
        let contents: Vec<String> = req.memories.iter().map(|m| m.memory.clone()).collect();
        let embeddings = match self.embedder.embed_batch(&contents).await {
            Ok(emb) => emb,
            Err(e) => {
                return Ok(BatchAddResponse {
                    code: 500,
                    message: format!("embedding failed: {}", e),
                    data: Some(BatchAddData {
                        successful: vec![],
                        failed: req
                            .memories
                            .iter()
                            .enumerate()
                            .map(|(i, _)| BatchFailure {
                                index: i as u32,
                                error: format!("embedding failed: {}", e),
                            })
                            .collect(),
                        total: req.memories.len() as u32,
                    }),
                });
            }
        };

        // Add each memory
        for (idx, (content, emb)) in req.memories.iter().zip(embeddings).enumerate() {
            let id = Uuid::new_v4().to_string();
            let mut metadata = HashMap::new();
            metadata.insert(
                "scope".to_string(),
                serde_json::Value::String(self.default_scope.clone()),
            );
            metadata.insert(
                "created_at".to_string(),
                serde_json::Value::String(Utc::now().to_rfc3339()),
            );

            if let Some(ref meta) = content.metadata {
                for (k, v) in meta {
                    metadata.insert(k.clone(), v.clone());
                }
            }

            let scope = content.scope.as_deref().unwrap_or(&self.default_scope);
            metadata.insert(
                "scope".to_string(),
                serde_json::Value::String(scope.to_string()),
            );

            let node = MemoryNode {
                id: id.clone(),
                memory: content.memory.clone(),
                metadata: metadata.clone(),
                embedding: Some(emb.clone()),
            };

            // Write to graph
            if let Err(e) = self.graph.add_nodes_batch(&[node], Some(user_name)).await {
                failed.push(BatchFailure {
                    index: idx as u32,
                    error: format!("graph error: {}", e),
                });
                continue;
            }

            // Write to vector store
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
                    serde_json::Value::String(scope.to_string()),
                );
                p
            };
            let item = VecStoreItem {
                id: id.clone(),
                vector: emb,
                payload,
            };

            if let Err(e) = self.vec_store.add(&[item], None).await {
                let _ = self.graph.delete_node(&id, Some(user_name)).await;
                failed.push(BatchFailure {
                    index: idx as u32,
                    error: format!("vector store error: {}", e),
                });
                continue;
            }

            // Index to keyword store if available
            if let Some(ref kw) = self.keyword_store {
                let _ = kw.index(&id, &content.memory, Some(user_name)).await;
            }

            successful.push(BatchResult {
                memory_id: id,
                index: idx as u32,
            });
        }

        Ok(BatchAddResponse {
            code: 200,
            message: "Batch add completed".to_string(),
            data: Some(BatchAddData {
                successful,
                failed,
                total: req.memories.len() as u32,
            }),
        })
    }

    async fn delete_memories_batch(
        &self,
        req: &BatchDeleteRequest,
    ) -> Result<BatchDeleteResponse, MemCubeError> {
        let user_name = req.mem_cube_id.as_deref().unwrap_or(req.user_id.as_str());

        let mut successful = Vec::new();
        let mut failed = Vec::new();

        for (idx, memory_id) in req.memory_ids.iter().enumerate() {
            // Check ownership
            let existing = self
                .graph
                .get_node(memory_id, false)
                .await
                .map_err(MemCubeError::Graph)?;
            if let Some(node) = existing {
                let node_owner = Self::node_owner(&node.metadata);
                if node_owner != user_name {
                    failed.push(BatchFailure {
                        index: idx as u32,
                        error: "memory not found".to_string(),
                    });
                    continue;
                }
            } else {
                failed.push(BatchFailure {
                    index: idx as u32,
                    error: "memory not found".to_string(),
                });
                continue;
            }

            if req.soft {
                // Soft delete
                let mut fields = HashMap::new();
                fields.insert(
                    "state".to_string(),
                    serde_json::Value::String("tombstone".to_string()),
                );
                fields.insert(
                    "updated_at".to_string(),
                    serde_json::Value::String(Utc::now().to_rfc3339()),
                );
                if let Err(e) = self
                    .graph
                    .update_node(memory_id, &fields, Some(user_name))
                    .await
                {
                    failed.push(BatchFailure {
                        index: idx as u32,
                        error: format!("update error: {}", e),
                    });
                    continue;
                }
                let _ = self.vec_store.delete(&[memory_id.clone()], None).await;
            } else {
                // Hard delete
                if let Err(e) = self.graph.delete_node(memory_id, Some(user_name)).await {
                    failed.push(BatchFailure {
                        index: idx as u32,
                        error: format!("delete error: {}", e),
                    });
                    continue;
                }
                let _ = self.vec_store.delete(&[memory_id.clone()], None).await;
            }

            if let Some(ref kw) = self.keyword_store {
                let _ = kw.remove(memory_id, Some(user_name)).await;
            }

            successful.push(BatchResult {
                memory_id: memory_id.clone(),
                index: idx as u32,
            });
        }

        Ok(BatchDeleteResponse {
            code: 200,
            message: "Batch delete completed".to_string(),
            data: Some(BatchAddData {
                successful,
                failed,
                total: req.memory_ids.len() as u32,
            }),
        })
    }

    async fn export_memories(&self, req: &ExportRequest) -> Result<ExportResponse, MemCubeError> {
        let user_name = req.mem_cube_id.as_deref().unwrap_or(req.user_id.as_str());

        // Get all memories for this user
        let all_nodes = self
            .graph
            .get_all_memory_items("all", user_name, false)
            .await
            .map_err(MemCubeError::Graph)?;

        // Filter by scope if specified
        let filtered: Vec<MemoryNode> = if req.scope == "all" {
            all_nodes
        } else {
            all_nodes
                .into_iter()
                .filter(|n| {
                    n.metadata
                        .get("scope")
                        .and_then(|v| v.as_str())
                        .map(|s| {
                            let normalized = s.to_lowercase().replace("memory", "");
                            normalized == req.scope.to_lowercase()
                        })
                        .unwrap_or(false)
                })
                .collect()
        };

        // Convert to MemoryItem
        let memories: Vec<MemoryItem> = filtered
            .into_iter()
            .map(|n| MemoryItem {
                id: n.id,
                memory: n.memory,
                metadata: n.metadata,
            })
            .collect();

        let total = memories.len() as u32;

        // Serialize to requested format
        let data = match req.format.as_str() {
            "jsonl" => memories
                .iter()
                .map(|m| serde_json::to_string(m).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => {
                // Default to JSON
                serde_json::to_string(&memories).unwrap_or_default()
            }
        };

        Ok(ExportResponse {
            code: 200,
            message: "Export completed".to_string(),
            data: Some(ExportData {
                total_memories: total,
                data,
            }),
        })
    }

    // ============================================================================
    // Session Management (P1-3)
    // ============================================================================

    async fn create_session(
        &self,
        req: &CreateSessionRequest,
    ) -> Result<SessionResponse, MemCubeError> {
        let session_store = self
            .session_store
            .as_ref()
            .ok_or_else(|| MemCubeError::Other("session store not configured".to_string()))?;

        let session = session_store
            .create_session(&req.user_id, req.title.as_deref(), req.metadata.as_ref())
            .await
            .map_err(|e| MemCubeError::Other(e.to_string()))?;

        Ok(SessionResponse {
            session_id: session.session_id,
            title: session.title,
            memory_count: session.memory_count,
            created_at: session.created_at,
            updated_at: session.updated_at,
            metadata: session.metadata,
        })
    }

    async fn get_session(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<Option<SessionResponse>, MemCubeError> {
        let session_store = self
            .session_store
            .as_ref()
            .ok_or_else(|| MemCubeError::Other("session store not configured".to_string()))?;

        let session = session_store
            .get_session(session_id, user_id)
            .await
            .map_err(|e| MemCubeError::Other(e.to_string()))?;

        Ok(session.map(|s| SessionResponse {
            session_id: s.session_id,
            title: s.title,
            memory_count: s.memory_count,
            created_at: s.created_at,
            updated_at: s.updated_at,
            metadata: s.metadata,
        }))
    }

    async fn list_sessions(
        &self,
        req: &ListSessionsRequest,
    ) -> Result<ListSessionsResponse, MemCubeError> {
        let session_store = self
            .session_store
            .as_ref()
            .ok_or_else(|| MemCubeError::Other("session store not configured".to_string()))?;

        let (sessions, cursor) = session_store
            .list_sessions(&req.user_id, req.limit, req.cursor.as_deref())
            .await
            .map_err(|e| MemCubeError::Other(e.to_string()))?;

        Ok(ListSessionsResponse {
            code: 200,
            message: "Success".to_string(),
            data: Some(ListSessionsData {
                sessions: sessions
                    .into_iter()
                    .map(|s| SessionResponse {
                        session_id: s.session_id,
                        title: s.title,
                        memory_count: s.memory_count,
                        created_at: s.created_at,
                        updated_at: s.updated_at,
                        metadata: s.metadata,
                    })
                    .collect(),
                next_cursor: cursor,
            }),
        })
    }

    async fn delete_session(
        &self,
        req: &DeleteSessionRequest,
    ) -> Result<MemoryResponse, MemCubeError> {
        let session_store = self
            .session_store
            .as_ref()
            .ok_or_else(|| MemCubeError::Other("session store not configured".to_string()))?;

        // Delete session
        session_store
            .delete_session(&req.session_id, &req.user_id)
            .await
            .map_err(|e| MemCubeError::Other(e.to_string()))?;

        // Optionally delete all memories in the session
        if req.delete_memories {
            // Get all memories with this session_id and delete them
            // This would require a full scan, which is inefficient
            // In production, you'd want a more efficient approach
            tracing::warn!("delete_memories not fully implemented for sessions");
        }

        Ok(MemoryResponse {
            code: 200,
            message: "Session deleted".to_string(),
            data: Some(vec![serde_json::json!({ "session_id": req.session_id })]),
        })
    }

    async fn session_timeline(
        &self,
        req: &SessionTimelineRequest,
    ) -> Result<SessionTimelineResponse, MemCubeError> {
        let user_name = &req.user_id;

        // Get all memories for this user and filter by session_id
        let all_nodes = self
            .graph
            .get_all_memory_items("all", user_name, false)
            .await
            .map_err(MemCubeError::Graph)?;

        let filtered: Vec<MemoryNode> = all_nodes
            .into_iter()
            .filter(|n| {
                n.metadata
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s == req.session_id)
                    .unwrap_or(false)
            })
            .collect();

        let total = filtered.len() as u32;

        // Apply limit
        let limited: Vec<MemoryItem> = filtered
            .into_iter()
            .take(req.limit as usize)
            .map(|n| MemoryItem {
                id: n.id,
                memory: n.memory,
                metadata: n.metadata,
            })
            .collect();

        Ok(SessionTimelineResponse {
            code: 200,
            message: "Success".to_string(),
            data: Some(SessionTimelineData {
                session_id: req.session_id.clone(),
                memories: limited,
                total,
            }),
        })
    }

    // ============================================================================
    // Memory Summary (P1-1)
    // ============================================================================

    async fn summarize_memories(
        &self,
        req: &SummarizeRequest,
    ) -> Result<SummarizeResponse, MemCubeError> {
        let llm_client = self
            .llm_client
            .as_ref()
            .ok_or_else(|| MemCubeError::Other("LLM client not configured".to_string()))?;

        let user_name = req.mem_cube_id.as_deref().unwrap_or(req.user_id.as_str());

        // Get memories to summarize
        let nodes = if let Some(ref memory_ids) = req.memory_ids {
            self.graph
                .get_nodes(memory_ids, false)
                .await
                .map_err(MemCubeError::Graph)?
        } else if let Some(ref session_id) = req.session_id {
            let all_nodes = self
                .graph
                .get_all_memory_items("all", user_name, false)
                .await
                .map_err(MemCubeError::Graph)?;
            all_nodes
                .into_iter()
                .filter(|n| {
                    n.metadata
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s == session_id)
                        .unwrap_or(false)
                })
                .collect()
        } else {
            return Err(MemCubeError::BadRequest(
                "need memory_ids or session_id".to_string(),
            ));
        };

        if nodes.is_empty() {
            return Err(MemCubeError::NotFound("no memories found".to_string()));
        }

        // Build prompt for summarization
        let content = nodes
            .iter()
            .map(|n| n.memory.as_str())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let prompt = format!(
            "请将以下记忆精简为不超过{}字的摘要，保留关键信息：\n\n{}",
            req.max_words, content
        );

        // Call LLM
        let summary = llm_client
            .complete(&prompt)
            .await
            .map_err(|e| MemCubeError::Other(format!("LLM error: {}", e)))?;

        // Save summary as a new memory
        let id = Uuid::new_v4().to_string();
        let embedding = self
            .embedder
            .embed(&summary)
            .await
            .map_err(MemCubeError::Embedder)?;

        let mut metadata = HashMap::new();
        metadata.insert(
            "scope".to_string(),
            serde_json::Value::String("LongTermMemory".to_string()),
        );
        metadata.insert(
            "created_at".to_string(),
            serde_json::Value::String(Utc::now().to_rfc3339()),
        );
        metadata.insert("is_summary".to_string(), serde_json::Value::Bool(true));
        if let Some(ref session_id) = req.session_id {
            metadata.insert(
                "session_id".to_string(),
                serde_json::Value::String(session_id.clone()),
            );
        }
        metadata.insert(
            "summarized_count".to_string(),
            serde_json::Value::Number(nodes.len().into()),
        );

        let node = MemoryNode {
            id: id.clone(),
            memory: summary.clone(),
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
            p.insert(
                "scope".to_string(),
                serde_json::Value::String("LongTermMemory".to_string()),
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

        Ok(SummarizeResponse {
            code: 200,
            message: "Summary created".to_string(),
            data: Some(SummarizeData {
                summary,
                summary_memory_id: id,
                summarized_count: nodes.len() as u32,
            }),
        })
    }
}
