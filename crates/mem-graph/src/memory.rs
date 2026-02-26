//! In-memory graph store with KNN search over embeddings.

use mem_types::{
    GraphDirection, GraphNeighbor, GraphPath, GraphStore, GraphStoreError, MemoryEdge, MemoryNode,
    VecSearchHit,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;

type ScopeIndex = HashMap<String, HashMap<String, Vec<String>>>;
type EdgeIndex = HashMap<String, Vec<String>>;

/// In-memory implementation of GraphStore.
/// Nodes are keyed by id (globally unique); user/scope indexed for get_all_memory_items and search filtering.
pub struct InMemoryGraphStore {
    /// node_id -> node (embedding optional; used for search_by_embedding when present).
    nodes: Arc<RwLock<HashMap<String, MemoryNode>>>,
    /// user_name -> scope -> node_ids (for get_all_memory_items).
    scope_index: Arc<RwLock<ScopeIndex>>,
    /// edge_id -> edge.
    edges: Arc<RwLock<HashMap<String, MemoryEdge>>>,
    /// from_node_id -> edge_ids.
    out_index: Arc<RwLock<EdgeIndex>>,
    /// to_node_id -> edge_ids.
    in_index: Arc<RwLock<EdgeIndex>>,
}

impl InMemoryGraphStore {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            scope_index: Arc::new(RwLock::new(HashMap::new())),
            edges: Arc::new(RwLock::new(HashMap::new())),
            out_index: Arc::new(RwLock::new(HashMap::new())),
            in_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn scope_for_node(metadata: &HashMap<String, serde_json::Value>) -> String {
        metadata
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("LongTermMemory")
            .to_string()
    }

    fn owner_from_metadata(metadata: &HashMap<String, serde_json::Value>) -> &str {
        metadata
            .get("user_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }

    fn add_edge_to_index(index: &mut EdgeIndex, node_id: &str, edge_id: &str) {
        let list = index.entry(node_id.to_string()).or_default();
        if !list.contains(&edge_id.to_string()) {
            list.push(edge_id.to_string());
        }
    }

    fn remove_edge_from_index(index: &mut EdgeIndex, node_id: &str, edge_id: &str) {
        if let Some(list) = index.get_mut(node_id) {
            list.retain(|x| x != edge_id);
            if list.is_empty() {
                index.remove(node_id);
            }
        }
    }

    fn add_edge_indexes(edge: &MemoryEdge, out_index: &mut EdgeIndex, in_index: &mut EdgeIndex) {
        Self::add_edge_to_index(out_index, &edge.from, &edge.id);
        Self::add_edge_to_index(in_index, &edge.to, &edge.id);
    }

    fn remove_edge_indexes(edge: &MemoryEdge, out_index: &mut EdgeIndex, in_index: &mut EdgeIndex) {
        Self::remove_edge_from_index(out_index, &edge.from, &edge.id);
        Self::remove_edge_from_index(in_index, &edge.to, &edge.id);
    }

    fn strip_embedding(mut node: MemoryNode, include_embedding: bool) -> MemoryNode {
        if !include_embedding {
            node.embedding = None;
        }
        node
    }

    fn is_tombstone(metadata: &HashMap<String, serde_json::Value>) -> bool {
        metadata
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("active")
            == "tombstone"
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

    async fn add_edges_batch(
        &self,
        edges: &[MemoryEdge],
        user_name: Option<&str>,
    ) -> Result<(), GraphStoreError> {
        if edges.is_empty() {
            return Ok(());
        }
        {
            let nodes = self.nodes.read().await;
            for edge in edges {
                let from_node = nodes.get(&edge.from).ok_or_else(|| {
                    GraphStoreError::Other(format!("from node not found: {}", edge.from))
                })?;
                let to_node = nodes.get(&edge.to).ok_or_else(|| {
                    GraphStoreError::Other(format!("to node not found: {}", edge.to))
                })?;
                if let Some(un) = user_name {
                    if Self::owner_from_metadata(&from_node.metadata) != un
                        || Self::owner_from_metadata(&to_node.metadata) != un
                    {
                        return Err(GraphStoreError::Other(format!(
                            "node not found or access denied for edge: {}",
                            edge.id
                        )));
                    }
                }
            }
        }

        let un = user_name.unwrap_or("");
        let mut edge_guard = self.edges.write().await;
        let mut out_guard = self.out_index.write().await;
        let mut in_guard = self.in_index.write().await;
        for edge in edges {
            let mut normalized = edge.clone();
            normalized.metadata.insert(
                "user_name".to_string(),
                serde_json::Value::String(un.to_string()),
            );
            if let Some(old) = edge_guard.insert(normalized.id.clone(), normalized.clone()) {
                Self::remove_edge_indexes(&old, &mut out_guard, &mut in_guard);
            }
            Self::add_edge_indexes(&normalized, &mut out_guard, &mut in_guard);
        }
        Ok(())
    }

    async fn get_node(
        &self,
        id: &str,
        include_embedding: bool,
    ) -> Result<Option<MemoryNode>, GraphStoreError> {
        let guard = self.nodes.read().await;
        Ok(guard
            .get(id)
            .cloned()
            .map(|n| Self::strip_embedding(n, include_embedding)))
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
                result.push(Self::strip_embedding(node.clone(), include_embedding));
            }
        }
        Ok(result)
    }

    async fn get_neighbors(
        &self,
        id: &str,
        relation: Option<&str>,
        direction: GraphDirection,
        limit: usize,
        include_embedding: bool,
        user_name: Option<&str>,
    ) -> Result<Vec<GraphNeighbor>, GraphStoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        {
            let nodes = self.nodes.read().await;
            let node = nodes
                .get(id)
                .ok_or_else(|| GraphStoreError::Other(format!("node not found: {}", id)))?;
            if let Some(un) = user_name {
                if Self::owner_from_metadata(&node.metadata) != un {
                    return Err(GraphStoreError::Other(format!(
                        "node not found or access denied: {}",
                        id
                    )));
                }
            }
        }

        let mut edge_ids: Vec<String> = Vec::new();
        match direction {
            GraphDirection::Outbound => {
                let out_guard = self.out_index.read().await;
                edge_ids.extend(out_guard.get(id).cloned().unwrap_or_default());
            }
            GraphDirection::Inbound => {
                let in_guard = self.in_index.read().await;
                edge_ids.extend(in_guard.get(id).cloned().unwrap_or_default());
            }
            GraphDirection::Both => {
                let out_guard = self.out_index.read().await;
                edge_ids.extend(out_guard.get(id).cloned().unwrap_or_default());
                let in_guard = self.in_index.read().await;
                edge_ids.extend(in_guard.get(id).cloned().unwrap_or_default());
            }
        }
        if edge_ids.is_empty() {
            return Ok(Vec::new());
        }

        let edge_guard = self.edges.read().await;
        let node_guard = self.nodes.read().await;
        let mut visited = HashSet::new();
        let mut edges_to_visit: Vec<MemoryEdge> = Vec::new();

        for edge_id in edge_ids {
            if !visited.insert(edge_id.clone()) {
                continue;
            }
            let edge = match edge_guard.get(&edge_id) {
                Some(e) => e.clone(),
                None => continue,
            };
            if let Some(un) = user_name {
                if Self::owner_from_metadata(&edge.metadata) != un {
                    continue;
                }
            }
            if let Some(rel) = relation {
                if edge.relation != rel {
                    continue;
                }
            }
            edges_to_visit.push(edge);
        }
        // Keep traversal deterministic across runtimes and hash-map ordering.
        edges_to_visit.sort_by(|a, b| a.id.cmp(&b.id));

        let mut result = Vec::new();
        for edge in edges_to_visit {
            if result.len() >= limit {
                break;
            }
            let neighbor_id = match direction {
                GraphDirection::Outbound => {
                    if edge.from == id {
                        &edge.to
                    } else {
                        continue;
                    }
                }
                GraphDirection::Inbound => {
                    if edge.to == id {
                        &edge.from
                    } else {
                        continue;
                    }
                }
                GraphDirection::Both => {
                    if edge.from == id {
                        &edge.to
                    } else if edge.to == id {
                        &edge.from
                    } else {
                        continue;
                    }
                }
            };

            let neighbor_node = match node_guard.get(neighbor_id) {
                Some(n) => n,
                None => continue,
            };
            if let Some(un) = user_name {
                if Self::owner_from_metadata(&neighbor_node.metadata) != un {
                    continue;
                }
            }

            result.push(GraphNeighbor {
                edge,
                node: Self::strip_embedding(neighbor_node.clone(), include_embedding),
            });
        }
        Ok(result)
    }

    async fn shortest_path(
        &self,
        source_id: &str,
        target_id: &str,
        relation: Option<&str>,
        direction: GraphDirection,
        max_depth: usize,
        include_deleted: bool,
        user_name: Option<&str>,
    ) -> Result<Option<GraphPath>, GraphStoreError> {
        if max_depth == 0 && source_id != target_id {
            return Ok(None);
        }

        {
            let nodes = self.nodes.read().await;
            let source = nodes
                .get(source_id)
                .ok_or_else(|| GraphStoreError::Other(format!("node not found: {}", source_id)))?;
            let target = nodes
                .get(target_id)
                .ok_or_else(|| GraphStoreError::Other(format!("node not found: {}", target_id)))?;
            if let Some(un) = user_name {
                if Self::owner_from_metadata(&source.metadata) != un
                    || Self::owner_from_metadata(&target.metadata) != un
                {
                    return Err(GraphStoreError::Other(format!(
                        "node not found or access denied: {} -> {}",
                        source_id, target_id
                    )));
                }
            }
            if !include_deleted
                && (Self::is_tombstone(&source.metadata) || Self::is_tombstone(&target.metadata))
            {
                return Ok(None);
            }
        }

        if source_id == target_id {
            return Ok(Some(GraphPath {
                node_ids: vec![source_id.to_string()],
                edges: Vec::new(),
            }));
        }

        let edge_guard = self.edges.read().await;
        let node_guard = self.nodes.read().await;
        let out_guard = self.out_index.read().await;
        let in_guard = self.in_index.read().await;

        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut prev: HashMap<String, (String, MemoryEdge)> = HashMap::new();

        queue.push_back((source_id.to_string(), 0));
        visited.insert(source_id.to_string());

        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            let mut transitions: Vec<(String, MemoryEdge)> = Vec::new();
            let mut edge_ids: Vec<String> = Vec::new();
            match direction {
                GraphDirection::Outbound => {
                    edge_ids.extend(out_guard.get(&current).cloned().unwrap_or_default());
                }
                GraphDirection::Inbound => {
                    edge_ids.extend(in_guard.get(&current).cloned().unwrap_or_default());
                }
                GraphDirection::Both => {
                    edge_ids.extend(out_guard.get(&current).cloned().unwrap_or_default());
                    edge_ids.extend(in_guard.get(&current).cloned().unwrap_or_default());
                }
            }

            let mut dedup = HashSet::new();
            for edge_id in edge_ids {
                if !dedup.insert(edge_id.clone()) {
                    continue;
                }
                let edge = match edge_guard.get(&edge_id) {
                    Some(e) => e.clone(),
                    None => continue,
                };
                if let Some(un) = user_name {
                    if Self::owner_from_metadata(&edge.metadata) != un {
                        continue;
                    }
                }
                if let Some(rel) = relation {
                    if edge.relation != rel {
                        continue;
                    }
                }

                let next = match direction {
                    GraphDirection::Outbound => {
                        if edge.from == current {
                            Some(edge.to.clone())
                        } else {
                            None
                        }
                    }
                    GraphDirection::Inbound => {
                        if edge.to == current {
                            Some(edge.from.clone())
                        } else {
                            None
                        }
                    }
                    GraphDirection::Both => {
                        if edge.from == current {
                            Some(edge.to.clone())
                        } else if edge.to == current {
                            Some(edge.from.clone())
                        } else {
                            None
                        }
                    }
                };
                let Some(next_node_id) = next else { continue };
                let Some(next_node) = node_guard.get(&next_node_id) else {
                    continue;
                };
                if let Some(un) = user_name {
                    if Self::owner_from_metadata(&next_node.metadata) != un {
                        continue;
                    }
                }
                if !include_deleted && Self::is_tombstone(&next_node.metadata) {
                    continue;
                }
                transitions.push((next_node_id, edge));
            }

            transitions.sort_by(|a, b| a.1.id.cmp(&b.1.id).then_with(|| a.0.cmp(&b.0)));

            for (next_node_id, edge) in transitions {
                if visited.contains(&next_node_id) {
                    continue;
                }
                visited.insert(next_node_id.clone());
                prev.insert(next_node_id.clone(), (current.clone(), edge));
                if next_node_id == target_id {
                    let mut rev_nodes = vec![target_id.to_string()];
                    let mut rev_edges: Vec<MemoryEdge> = Vec::new();
                    let mut cursor = target_id.to_string();
                    while cursor != source_id {
                        let (p, e) = prev.get(&cursor).ok_or_else(|| {
                            GraphStoreError::Other("path reconstruction failed".to_string())
                        })?;
                        rev_edges.push(e.clone());
                        rev_nodes.push(p.clone());
                        cursor = p.clone();
                    }
                    rev_nodes.reverse();
                    rev_edges.reverse();
                    return Ok(Some(GraphPath {
                        node_ids: rev_nodes,
                        edges: rev_edges,
                    }));
                }
                queue.push_back((next_node_id, depth + 1));
            }
        }

        Ok(None)
    }

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
    ) -> Result<Vec<GraphPath>, GraphStoreError> {
        if top_k == 0 {
            return Ok(Vec::new());
        }
        if max_depth == 0 && source_id != target_id {
            return Ok(Vec::new());
        }

        {
            let nodes = self.nodes.read().await;
            let source = nodes
                .get(source_id)
                .ok_or_else(|| GraphStoreError::Other(format!("node not found: {}", source_id)))?;
            let target = nodes
                .get(target_id)
                .ok_or_else(|| GraphStoreError::Other(format!("node not found: {}", target_id)))?;
            if let Some(un) = user_name {
                if Self::owner_from_metadata(&source.metadata) != un
                    || Self::owner_from_metadata(&target.metadata) != un
                {
                    return Err(GraphStoreError::Other(format!(
                        "node not found or access denied: {} -> {}",
                        source_id, target_id
                    )));
                }
            }
            if !include_deleted
                && (Self::is_tombstone(&source.metadata) || Self::is_tombstone(&target.metadata))
            {
                return Ok(Vec::new());
            }
        }

        if source_id == target_id {
            return Ok(vec![GraphPath {
                node_ids: vec![source_id.to_string()],
                edges: Vec::new(),
            }]);
        }

        #[derive(Clone)]
        struct PathState {
            current: String,
            node_ids: Vec<String>,
            edges: Vec<MemoryEdge>,
            visited: HashSet<String>,
        }

        let edge_guard = self.edges.read().await;
        let node_guard = self.nodes.read().await;
        let out_guard = self.out_index.read().await;
        let in_guard = self.in_index.read().await;

        let mut queue: VecDeque<PathState> = VecDeque::new();
        let mut start_visited = HashSet::new();
        start_visited.insert(source_id.to_string());
        queue.push_back(PathState {
            current: source_id.to_string(),
            node_ids: vec![source_id.to_string()],
            edges: Vec::new(),
            visited: start_visited,
        });

        let mut results: Vec<GraphPath> = Vec::new();
        while let Some(state) = queue.pop_front() {
            if results.len() >= top_k {
                break;
            }
            if state.current == target_id {
                results.push(GraphPath {
                    node_ids: state.node_ids.clone(),
                    edges: state.edges.clone(),
                });
                continue;
            }
            if state.edges.len() >= max_depth {
                continue;
            }

            let mut edge_ids: Vec<String> = Vec::new();
            match direction {
                GraphDirection::Outbound => {
                    edge_ids.extend(out_guard.get(&state.current).cloned().unwrap_or_default());
                }
                GraphDirection::Inbound => {
                    edge_ids.extend(in_guard.get(&state.current).cloned().unwrap_or_default());
                }
                GraphDirection::Both => {
                    edge_ids.extend(out_guard.get(&state.current).cloned().unwrap_or_default());
                    edge_ids.extend(in_guard.get(&state.current).cloned().unwrap_or_default());
                }
            }

            let mut dedup = HashSet::new();
            let mut transitions: Vec<(String, MemoryEdge)> = Vec::new();
            for edge_id in edge_ids {
                if !dedup.insert(edge_id.clone()) {
                    continue;
                }
                let edge = match edge_guard.get(&edge_id) {
                    Some(e) => e.clone(),
                    None => continue,
                };
                if let Some(un) = user_name {
                    if Self::owner_from_metadata(&edge.metadata) != un {
                        continue;
                    }
                }
                if let Some(rel) = relation {
                    if edge.relation != rel {
                        continue;
                    }
                }

                let next = match direction {
                    GraphDirection::Outbound => {
                        if edge.from == state.current {
                            Some(edge.to.clone())
                        } else {
                            None
                        }
                    }
                    GraphDirection::Inbound => {
                        if edge.to == state.current {
                            Some(edge.from.clone())
                        } else {
                            None
                        }
                    }
                    GraphDirection::Both => {
                        if edge.from == state.current {
                            Some(edge.to.clone())
                        } else if edge.to == state.current {
                            Some(edge.from.clone())
                        } else {
                            None
                        }
                    }
                };

                let Some(next_node_id) = next else { continue };
                if state.visited.contains(&next_node_id) {
                    continue;
                }
                let Some(next_node) = node_guard.get(&next_node_id) else {
                    continue;
                };
                if let Some(un) = user_name {
                    if Self::owner_from_metadata(&next_node.metadata) != un {
                        continue;
                    }
                }
                if !include_deleted && Self::is_tombstone(&next_node.metadata) {
                    continue;
                }
                transitions.push((next_node_id, edge));
            }
            transitions.sort_by(|a, b| a.1.id.cmp(&b.1.id).then_with(|| a.0.cmp(&b.0)));

            for (next_node_id, edge) in transitions {
                let mut next_state = state.clone();
                next_state.current = next_node_id.clone();
                next_state.node_ids.push(next_node_id.clone());
                next_state.edges.push(edge);
                next_state.visited.insert(next_node_id);
                queue.push_back(next_state);
            }
        }

        Ok(results)
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
                let node_user = Self::owner_from_metadata(&node.metadata);
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
            let node_owner = Self::owner_from_metadata(&node.metadata);
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
                let node_owner = Self::owner_from_metadata(&node.metadata);
                if node_owner != un {
                    return Err(GraphStoreError::Other(format!(
                        "node not found or access denied: {}",
                        id
                    )));
                }
            }
        }
        {
            let mut nodes = self.nodes.write().await;
            nodes
                .remove(id)
                .ok_or_else(|| GraphStoreError::Other(format!("node not found: {}", id)))?;
        }
        {
            let mut idx = self.scope_index.write().await;
            for scope_map in idx.values_mut() {
                for list in scope_map.values_mut() {
                    list.retain(|x| x != id);
                }
            }
        }
        self.delete_edges_by_node(id, user_name).await?;
        Ok(())
    }

    async fn delete_edges_by_node(
        &self,
        id: &str,
        user_name: Option<&str>,
    ) -> Result<usize, GraphStoreError> {
        let mut edge_ids: HashSet<String> = HashSet::new();
        {
            let out_guard = self.out_index.read().await;
            edge_ids.extend(out_guard.get(id).cloned().unwrap_or_default());
        }
        {
            let in_guard = self.in_index.read().await;
            edge_ids.extend(in_guard.get(id).cloned().unwrap_or_default());
        }

        if edge_ids.is_empty() {
            return Ok(0);
        }

        let mut edge_guard = self.edges.write().await;
        let mut out_guard = self.out_index.write().await;
        let mut in_guard = self.in_index.write().await;

        if let Some(un) = user_name {
            for edge_id in &edge_ids {
                if let Some(edge) = edge_guard.get(edge_id) {
                    if Self::owner_from_metadata(&edge.metadata) != un {
                        return Err(GraphStoreError::Other(format!(
                            "edge not found or access denied: {}",
                            edge_id
                        )));
                    }
                }
            }
        }

        let mut deleted = 0usize;
        for edge_id in edge_ids {
            if let Some(edge) = edge_guard.remove(&edge_id) {
                Self::remove_edge_indexes(&edge, &mut out_guard, &mut in_guard);
                deleted += 1;
            }
        }
        Ok(deleted)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn neighbors_are_deterministic_and_limited() {
        let store = InMemoryGraphStore::new();

        let mut meta = HashMap::new();
        meta.insert(
            "scope".to_string(),
            serde_json::Value::String("LongTermMemory".to_string()),
        );

        store
            .add_node("n0", "root", &meta, Some("u1"))
            .await
            .unwrap();
        store
            .add_node("n1", "node1", &meta, Some("u1"))
            .await
            .unwrap();
        store
            .add_node("n2", "node2", &meta, Some("u1"))
            .await
            .unwrap();

        store
            .add_edges_batch(
                &[
                    MemoryEdge {
                        id: "e2".to_string(),
                        from: "n0".to_string(),
                        to: "n2".to_string(),
                        relation: "related_to".to_string(),
                        metadata: HashMap::new(),
                    },
                    MemoryEdge {
                        id: "e1".to_string(),
                        from: "n0".to_string(),
                        to: "n1".to_string(),
                        relation: "related_to".to_string(),
                        metadata: HashMap::new(),
                    },
                ],
                Some("u1"),
            )
            .await
            .unwrap();

        let all = store
            .get_neighbors(
                "n0",
                Some("related_to"),
                GraphDirection::Outbound,
                10,
                false,
                Some("u1"),
            )
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].edge.id, "e1");
        assert_eq!(all[1].edge.id, "e2");

        let limited = store
            .get_neighbors(
                "n0",
                Some("related_to"),
                GraphDirection::Outbound,
                1,
                false,
                Some("u1"),
            )
            .await
            .unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].edge.id, "e1");
    }

    #[tokio::test]
    async fn shortest_path_finds_min_hops() {
        let store = InMemoryGraphStore::new();

        let mut meta = HashMap::new();
        meta.insert(
            "scope".to_string(),
            serde_json::Value::String("LongTermMemory".to_string()),
        );

        store.add_node("a", "A", &meta, Some("u1")).await.unwrap();
        store.add_node("b", "B", &meta, Some("u1")).await.unwrap();
        store.add_node("c", "C", &meta, Some("u1")).await.unwrap();
        store.add_node("d", "D", &meta, Some("u1")).await.unwrap();

        store
            .add_edges_batch(
                &[
                    MemoryEdge {
                        id: "e_ab".to_string(),
                        from: "a".to_string(),
                        to: "b".to_string(),
                        relation: "related_to".to_string(),
                        metadata: HashMap::new(),
                    },
                    MemoryEdge {
                        id: "e_bc".to_string(),
                        from: "b".to_string(),
                        to: "c".to_string(),
                        relation: "related_to".to_string(),
                        metadata: HashMap::new(),
                    },
                    MemoryEdge {
                        id: "e_ad".to_string(),
                        from: "a".to_string(),
                        to: "d".to_string(),
                        relation: "related_to".to_string(),
                        metadata: HashMap::new(),
                    },
                    MemoryEdge {
                        id: "e_dc".to_string(),
                        from: "d".to_string(),
                        to: "c".to_string(),
                        relation: "related_to".to_string(),
                        metadata: HashMap::new(),
                    },
                ],
                Some("u1"),
            )
            .await
            .unwrap();

        let path = store
            .shortest_path(
                "a",
                "c",
                Some("related_to"),
                GraphDirection::Outbound,
                3,
                false,
                Some("u1"),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(path.node_ids.first().map(String::as_str), Some("a"));
        assert_eq!(path.node_ids.last().map(String::as_str), Some("c"));
        assert_eq!(path.edges.len(), 2);
    }

    #[tokio::test]
    async fn find_paths_returns_top_k_shortest() {
        let store = InMemoryGraphStore::new();

        let mut meta = HashMap::new();
        meta.insert(
            "scope".to_string(),
            serde_json::Value::String("LongTermMemory".to_string()),
        );

        for id in ["s", "a", "b", "t"] {
            store.add_node(id, id, &meta, Some("u1")).await.unwrap();
        }
        store
            .add_edges_batch(
                &[
                    MemoryEdge {
                        id: "e_sa".to_string(),
                        from: "s".to_string(),
                        to: "a".to_string(),
                        relation: "r".to_string(),
                        metadata: HashMap::new(),
                    },
                    MemoryEdge {
                        id: "e_at".to_string(),
                        from: "a".to_string(),
                        to: "t".to_string(),
                        relation: "r".to_string(),
                        metadata: HashMap::new(),
                    },
                    MemoryEdge {
                        id: "e_sb".to_string(),
                        from: "s".to_string(),
                        to: "b".to_string(),
                        relation: "r".to_string(),
                        metadata: HashMap::new(),
                    },
                    MemoryEdge {
                        id: "e_bt".to_string(),
                        from: "b".to_string(),
                        to: "t".to_string(),
                        relation: "r".to_string(),
                        metadata: HashMap::new(),
                    },
                ],
                Some("u1"),
            )
            .await
            .unwrap();

        let paths = store
            .find_paths(
                "s",
                "t",
                Some("r"),
                GraphDirection::Outbound,
                3,
                2,
                false,
                Some("u1"),
            )
            .await
            .unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].edges.len(), 2);
        assert_eq!(paths[1].edges.len(), 2);
    }
}
