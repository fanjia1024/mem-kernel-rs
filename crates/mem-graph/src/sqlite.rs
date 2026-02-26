//! SQLite-backed graph store implementation (P0: persistence).

use crate::{
    GraphNeighbor, GraphPath, GraphStore, GraphStoreError, MemoryEdge, MemoryNode, VecSearchHit,
};
use async_trait::async_trait;
use mem_types::GraphDirection;
use std::collections::HashMap;
use std::path::Path;

/// SQLite-backed graph store for persistence.
pub struct SqliteGraphStore {
    conn: std::sync::Mutex<rusqlite::Connection>,
}

impl SqliteGraphStore {
    /// Create a new SQLite graph store at the given path.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, GraphStoreError> {
        let conn =
            rusqlite::Connection::open(path).map_err(|e| GraphStoreError::Other(e.to_string()))?;

        // Initialize schema
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS nodes (
                id TEXT PRIMARY KEY,
                memory TEXT NOT NULL,
                metadata TEXT NOT NULL,
                embedding BLOB,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS edges (
                id TEXT PRIMARY KEY,
                from_node TEXT NOT NULL,
                to_node TEXT NOT NULL,
                relation TEXT NOT NULL,
                metadata TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (from_node) REFERENCES nodes(id) ON DELETE CASCADE,
                FOREIGN KEY (to_node) REFERENCES nodes(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_nodes_user ON nodes(metadata);
            CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_node);
            CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_node);
            CREATE INDEX IF NOT EXISTS idx_edges_relation ON edges(relation);
            "#,
        )
        .map_err(|e| GraphStoreError::Other(e.to_string()))?;

        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    fn with_conn<T, F>(&self, f: F) -> Result<T, GraphStoreError>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<T, rusqlite::Error>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| GraphStoreError::Other(format!("failed to acquire lock: {}", e)))?;
        f(&conn).map_err(|e| GraphStoreError::Other(e.to_string()))
    }
}

#[async_trait]
impl GraphStore for SqliteGraphStore {
    async fn add_node(
        &self,
        id: &str,
        memory: &str,
        metadata: &HashMap<String, serde_json::Value>,
        _user_name: Option<&str>,
    ) -> Result<(), GraphStoreError> {
        let id = id.to_string();
        let memory = memory.to_string();
        let metadata_json =
            serde_json::to_string(metadata).map_err(|e| GraphStoreError::Other(e.to_string()))?;
        let now = chrono::Utc::now().to_rfc3339();

        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO nodes (id, memory, metadata, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, memory, metadata_json, now, now],
            )
        })
        .map_err(|e| GraphStoreError::Other(e.to_string()))?;

        Ok(())
    }

    async fn add_nodes_batch(
        &self,
        nodes: &[MemoryNode],
        _user_name: Option<&str>,
    ) -> Result<(), GraphStoreError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            for node in nodes {
                let metadata_json = serde_json::to_string(&node.metadata)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                tx.execute(
                    "INSERT OR REPLACE INTO nodes (id, memory, metadata, embedding, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        node.id,
                        node.memory,
                        metadata_json,
                        node.embedding.as_ref().and_then(|e| serde_json::to_vec(e).ok()),
                        now,
                        now,
                    ],
                )?;
            }
            tx.commit()
        })
        .map_err(|e| GraphStoreError::Other(e.to_string()))?;

        Ok(())
    }

    async fn add_edges_batch(
        &self,
        edges: &[MemoryEdge],
        _user_name: Option<&str>,
    ) -> Result<(), GraphStoreError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            for edge in edges {
                let metadata_json = serde_json::to_string(&edge.metadata)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                tx.execute(
                    "INSERT OR REPLACE INTO edges (id, from_node, to_node, relation, metadata, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        edge.id,
                        edge.from,
                        edge.to,
                        edge.relation,
                        metadata_json,
                        now,
                    ],
                )?;
            }
            tx.commit()
        })
        .map_err(|e| GraphStoreError::Other(e.to_string()))?;

        Ok(())
    }

    async fn get_node(
        &self,
        id: &str,
        _include_embedding: bool,
    ) -> Result<Option<MemoryNode>, GraphStoreError> {
        let id = id.to_string();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, memory, metadata, embedding, created_at, updated_at FROM nodes WHERE id = ?1",
            )?;
            let result = stmt.query_row([&id], |row| {
                let metadata_json: String = row.get(2)?;
                let embedding_blob: Option<Vec<u8>> = row.get(3)?;
                let embedding = embedding_blob
                    .and_then(|b| serde_json::from_slice(&b).ok());
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    metadata_json,
                    embedding,
                ))
            });

            match result {
                Ok((id, memory, metadata_json, embedding)) => {
                    let metadata: HashMap<String, serde_json::Value> =
                        serde_json::from_str(&metadata_json).unwrap_or_default();
                    Ok(Some(MemoryNode {
                        id,
                        memory,
                        metadata,
                        embedding,
                    }))
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
        .map_err(|e| GraphStoreError::Other(e.to_string()))
    }

    async fn get_nodes(
        &self,
        ids: &[String],
        _include_embedding: bool,
    ) -> Result<Vec<MemoryNode>, GraphStoreError> {
        if ids.is_empty() {
            return Ok(vec![]);
        }

        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT id, memory, metadata, embedding, created_at, updated_at FROM nodes WHERE id IN ({})",
            placeholders.join(",")
        );

        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let params: Vec<&dyn rusqlite::ToSql> =
                ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
            let rows = stmt.query_map(params.as_slice(), |row| {
                let metadata_json: String = row.get(2)?;
                let embedding_blob: Option<Vec<u8>> = row.get(3)?;
                let embedding = embedding_blob.and_then(|b| serde_json::from_slice(&b).ok());
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    metadata_json,
                    embedding,
                ))
            })?;

            let mut nodes = Vec::new();
            for row in rows {
                let (id, memory, metadata_json, embedding) = row?;
                let metadata: HashMap<String, serde_json::Value> =
                    serde_json::from_str(&metadata_json).unwrap_or_default();
                nodes.push(MemoryNode {
                    id,
                    memory,
                    metadata,
                    embedding,
                });
            }
            Ok(nodes)
        })
        .map_err(|e| GraphStoreError::Other(e.to_string()))
    }

    async fn get_neighbors(
        &self,
        id: &str,
        relation: Option<&str>,
        direction: GraphDirection,
        limit: usize,
        _include_embedding: bool,
        _user_name: Option<&str>,
    ) -> Result<Vec<GraphNeighbor>, GraphStoreError> {
        let id = id.to_string();
        let limit = limit as i64;

        let neighbors = self
            .with_conn(|conn| {
                let mut neighbors = Vec::new();

                // Get outbound edges (from -> to)
                if direction == GraphDirection::Outbound || direction == GraphDirection::Both {
                    // Always use two-parameter query for simplicity
                    let sql =
                        "SELECT e.id, e.from_node, e.to_node, e.relation, e.metadata, e.created_at,
                         n.id, n.memory, n.metadata, n.embedding, n.created_at, n.updated_at
                         FROM edges e JOIN nodes n ON e.to_node = n.id
                         WHERE e.from_node = ?1 AND (e.relation = ?2 OR ?2 IS NULL)";

                    let mut stmt = conn.prepare(sql)?;
                    let rel_dummy: Option<&str> = relation;
                    let rows = stmt.query_map(rusqlite::params![&id, rel_dummy], |row| {
                        // Extract all data from row immediately to avoid lifetime issues
                        let edge = MemoryEdge {
                            id: row.get(0)?,
                            from: row.get(1)?,
                            to: row.get(2)?,
                            relation: row.get(3)?,
                            metadata: serde_json::from_str(
                                &row.get::<_, String>(4).unwrap_or_default(),
                            )
                            .unwrap_or_default(),
                        };
                        let node = MemoryNode {
                            id: row.get(6)?,
                            memory: row.get(7)?,
                            metadata: serde_json::from_str(
                                &row.get::<_, String>(8).unwrap_or_default(),
                            )
                            .unwrap_or_default(),
                            embedding: row
                                .get::<_, Option<Vec<u8>>>(9)
                                .ok()
                                .flatten()
                                .and_then(|b| serde_json::from_slice(&b).ok()),
                        };
                        Ok((edge, node, true))
                    })?;

                    for row in rows {
                        let (edge, node, _) = row?;
                        neighbors.push(GraphNeighbor { edge, node });
                    }
                }

                // Get inbound edges (from -> to)
                if direction == GraphDirection::Inbound || direction == GraphDirection::Both {
                    let sql =
                        "SELECT e.id, e.from_node, e.to_node, e.relation, e.metadata, e.created_at,
                         n.id, n.memory, n.metadata, n.embedding, n.created_at, n.updated_at
                         FROM edges e JOIN nodes n ON e.from_node = n.id
                         WHERE e.to_node = ?1 AND (e.relation = ?2 OR ?2 IS NULL)";

                    let mut stmt = conn.prepare(sql)?;
                    let rel_dummy: Option<&str> = relation;
                    let rows = stmt.query_map(rusqlite::params![&id, rel_dummy], |row| {
                        // Extract all data from row immediately to avoid lifetime issues
                        let edge = MemoryEdge {
                            id: row.get(0)?,
                            from: row.get(1)?,
                            to: row.get(2)?,
                            relation: row.get(3)?,
                            metadata: serde_json::from_str(
                                &row.get::<_, String>(4).unwrap_or_default(),
                            )
                            .unwrap_or_default(),
                        };
                        let node = MemoryNode {
                            id: row.get(6)?,
                            memory: row.get(7)?,
                            metadata: serde_json::from_str(
                                &row.get::<_, String>(8).unwrap_or_default(),
                            )
                            .unwrap_or_default(),
                            embedding: row
                                .get::<_, Option<Vec<u8>>>(9)
                                .ok()
                                .flatten()
                                .and_then(|b| serde_json::from_slice(&b).ok()),
                        };
                        Ok((edge, node, false))
                    })?;

                    for row in rows {
                        let (edge, node, _) = row?;
                        neighbors.push(GraphNeighbor { edge, node });
                    }
                }

                Ok(neighbors)
            })
            .map_err(|e| GraphStoreError::Other(e.to_string()))?;

        // Apply limit
        let mut neighbors = neighbors;
        if limit > 0 && neighbors.len() > limit as usize {
            neighbors.truncate(limit as usize);
        }

        Ok(neighbors)
    }

    async fn shortest_path(
        &self,
        source_id: &str,
        target_id: &str,
        relation: Option<&str>,
        direction: GraphDirection,
        max_depth: usize,
        _include_deleted: bool,
        _user_name: Option<&str>,
    ) -> Result<Option<GraphPath>, GraphStoreError> {
        // Simple BFS for shortest path
        use std::collections::{HashSet, VecDeque};

        let source_id = source_id.to_string();
        let target_id = target_id.to_string();

        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, Vec<String>, Vec<MemoryEdge>)> = VecDeque::new();
        queue.push_back((source_id.clone(), vec![source_id.clone()], vec![]));
        visited.insert(source_id);

        while let Some((current_id, node_ids, edges)) = queue.pop_front() {
            if node_ids.len() > max_depth + 1 {
                continue;
            }

            if current_id == target_id {
                return Ok(Some(GraphPath { node_ids, edges }));
            }

            // Get neighbors
            let neighbors = self
                .get_neighbors(&current_id, relation, direction, 1000, false, None)
                .await?;

            for neighbor in neighbors {
                let next_id = neighbor.node.id.clone();
                if !visited.contains(&next_id) {
                    visited.insert(next_id.clone());
                    let mut new_node_ids = node_ids.clone();
                    new_node_ids.push(next_id.clone());
                    let mut new_edges = edges.clone();
                    new_edges.push(neighbor.edge);
                    queue.push_back((next_id, new_node_ids, new_edges));
                }
            }
        }

        Ok(None)
    }

    async fn find_paths(
        &self,
        source_id: &str,
        target_id: &str,
        _relation: Option<&str>,
        _direction: GraphDirection,
        max_depth: usize,
        top_k: usize,
        _include_deleted: bool,
        _user_name: Option<&str>,
    ) -> Result<Vec<GraphPath>, GraphStoreError> {
        // Simplified implementation: use shortest_path repeatedly
        let mut paths: Vec<GraphPath> = Vec::new();

        // For now, just call shortest_path which works
        if let Some(path) = self
            .shortest_path(
                source_id,
                target_id,
                None,
                GraphDirection::Both,
                max_depth,
                false,
                None,
            )
            .await?
        {
            paths.push(path);
        }

        // If we need more paths, we'd need a more complex implementation
        // For SQLite, we just return the first path found
        if paths.len() < top_k {
            // Could add more sophisticated path finding here
        }

        Ok(paths)
    }

    async fn search_by_embedding(
        &self,
        _vector: &[f32],
        _top_k: usize,
        _user_name: Option<&str>,
    ) -> Result<Vec<VecSearchHit>, GraphStoreError> {
        // SQLite doesn't support vector search natively
        // For production, use Qdrant or implement approximate nearest neighbor
        Err(GraphStoreError::Other(
            "vector search not supported in SQLite backend. Use QdrantVecStore instead."
                .to_string(),
        ))
    }

    async fn get_all_memory_items(
        &self,
        _scope: &str,
        _user_name: &str,
        _include_embedding: bool,
    ) -> Result<Vec<MemoryNode>, GraphStoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, memory, metadata, embedding, created_at, updated_at FROM nodes",
            )?;
            let rows = stmt.query_map([], |row| {
                let metadata_json: String = row.get(2)?;
                let embedding_blob: Option<Vec<u8>> = row.get(3)?;
                let embedding = embedding_blob
                    .and_then(|b| serde_json::from_slice(&b).ok());
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    metadata_json,
                    embedding,
                ))
            })?;

            let mut nodes = Vec::new();
            for row in rows {
                let (id, memory, metadata_json, embedding) = row?;
                let metadata: HashMap<String, serde_json::Value> =
                    serde_json::from_str(&metadata_json).unwrap_or_default();
                nodes.push(MemoryNode {
                    id,
                    memory,
                    metadata,
                    embedding,
                });
            }
            Ok(nodes)
        })
        .map_err(|e| GraphStoreError::Other(e.to_string()))
    }

    async fn update_node(
        &self,
        id: &str,
        fields: &HashMap<String, serde_json::Value>,
        _user_name: Option<&str>,
    ) -> Result<(), GraphStoreError> {
        let id = id.to_string();
        let _now = chrono::Utc::now().to_rfc3339();

        // Get existing node
        let existing = self.get_node(&id, false).await?;
        let existing =
            existing.ok_or_else(|| GraphStoreError::Other("node not found".to_string()))?;

        // Merge fields
        let mut metadata: HashMap<String, serde_json::Value> = existing.metadata;
        for (k, v) in fields {
            if k == "memory" {
                // Handle memory update separately
                continue;
            }
            metadata.insert(k.clone(), v.clone());
        }

        let memory = fields
            .get("memory")
            .and_then(|v| v.as_str())
            .unwrap_or(&existing.memory);

        self.add_node(&id, memory, &metadata, None)
            .await
            .map_err(|e| GraphStoreError::Other(e.to_string()))
    }

    async fn delete_node(&self, id: &str, _user_name: Option<&str>) -> Result<(), GraphStoreError> {
        let id = id.to_string();
        self.with_conn(|conn| {
            // Delete edges first
            conn.execute(
                "DELETE FROM edges WHERE from_node = ?1 OR to_node = ?1",
                [&id],
            )?;
            // Delete node
            conn.execute("DELETE FROM nodes WHERE id = ?1", [&id])?;
            Ok(())
        })
        .map_err(|e| GraphStoreError::Other(e.to_string()))?;

        Ok(())
    }

    async fn delete_edges_by_node(
        &self,
        id: &str,
        _user_name: Option<&str>,
    ) -> Result<usize, GraphStoreError> {
        let id = id.to_string();
        self.with_conn(|conn| {
            let count = conn.execute(
                "DELETE FROM edges WHERE from_node = ?1 OR to_node = ?1",
                [&id],
            )?;
            Ok(count)
        })
        .map_err(|e| GraphStoreError::Other(e.to_string()))
    }
}

#[allow(dead_code)]
fn parse_edge_row(row: &rusqlite::Row, offset: usize) -> Result<MemoryEdge, rusqlite::Error> {
    let metadata_json: String = row.get(offset + 4).unwrap_or_default();
    let metadata: HashMap<String, serde_json::Value> =
        serde_json::from_str(&metadata_json).unwrap_or_default();

    Ok(MemoryEdge {
        id: row.get(offset)?,
        from: row.get(offset + 1)?,
        to: row.get(offset + 2)?,
        relation: row.get(offset + 3)?,
        metadata,
    })
}

#[allow(dead_code)]
fn parse_node_row(row: &rusqlite::Row, offset: usize) -> Result<MemoryNode, rusqlite::Error> {
    let metadata_json: String = row.get(offset + 2).unwrap_or_default();
    let embedding_blob: Option<Vec<u8>> = row.get(offset + 3).ok();
    let embedding = embedding_blob.and_then(|b| serde_json::from_slice(&b).ok());

    let metadata: HashMap<String, serde_json::Value> =
        serde_json::from_str(&metadata_json).unwrap_or_default();

    Ok(MemoryNode {
        id: row.get(offset)?,
        memory: row.get(offset + 1)?,
        metadata,
        embedding,
    })
}
