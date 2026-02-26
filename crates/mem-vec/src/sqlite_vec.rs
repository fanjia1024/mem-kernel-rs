//! SQLite-backed vector store implementation (P0: persistence).
//! Note: Vector search is not natively supported in SQLite.
//! For production use with vector search, use QdrantVecStore.

use crate::{VecSearchHit, VecStore, VecStoreError, VecStoreItem};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;

/// SQLite-backed vector store for persistence (without native vector search).
pub struct SqliteVecStore {
    conn: std::sync::Mutex<rusqlite::Connection>,
}

impl SqliteVecStore {
    /// Create a new SQLite vector store at the given path.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, VecStoreError> {
        let conn =
            rusqlite::Connection::open(path).map_err(|e| VecStoreError::Other(e.to_string()))?;

        // Initialize schema
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS vectors (
                id TEXT PRIMARY KEY,
                vector BLOB NOT NULL,
                payload TEXT NOT NULL,
                collection TEXT DEFAULT 'default',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_vectors_collection ON vectors(collection);
            CREATE INDEX IF NOT EXISTS idx_vectors_id ON vectors(id);
            "#,
        )
        .map_err(|e| VecStoreError::Other(e.to_string()))?;

        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    fn with_conn<T, F>(&self, f: F) -> Result<T, VecStoreError>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<T, rusqlite::Error>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| VecStoreError::Other(format!("failed to acquire lock: {}", e)))?;
        f(&conn).map_err(|e| VecStoreError::Other(e.to_string()))
    }
}

#[async_trait]
impl VecStore for SqliteVecStore {
    async fn add(
        &self,
        items: &[VecStoreItem],
        collection: Option<&str>,
    ) -> Result<(), VecStoreError> {
        let coll = collection.unwrap_or("default");
        let now = chrono::Utc::now().to_rfc3339();

        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            for item in items {
                let vector_blob =
                    serde_json::to_vec(&item.vector).map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                    })?;
                let payload_json =
                    serde_json::to_string(&item.payload).map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                    })?;

                tx.execute(
                    "INSERT OR REPLACE INTO vectors (id, vector, payload, collection, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![item.id, vector_blob, payload_json, coll, now, now],
                )?;
            }
            tx.commit()
        })
        .map_err(|e| VecStoreError::Other(e.to_string()))?;

        Ok(())
    }

    async fn search(
        &self,
        _query_vector: &[f32],
        _top_k: usize,
        _filter: Option<&HashMap<String, serde_json::Value>>,
        _collection: Option<&str>,
    ) -> Result<Vec<VecSearchHit>, VecStoreError> {
        // SQLite doesn't support vector search natively
        // For production, use QdrantVecStore instead
        Err(VecStoreError::Other(
            "vector search not supported in SQLite backend. Use QdrantVecStore for vector search."
                .to_string(),
        ))
    }

    async fn get_by_ids(
        &self,
        ids: &[String],
        _collection: Option<&str>,
    ) -> Result<Vec<VecStoreItem>, VecStoreError> {
        if ids.is_empty() {
            return Ok(vec![]);
        }

        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT id, vector, payload, collection, created_at, updated_at FROM vectors WHERE id IN ({})",
            placeholders.join(",")
        );

        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let params: Vec<&dyn rusqlite::ToSql> =
                ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
            let rows = stmt.query_map(params.as_slice(), |row| {
                let vector_blob: Vec<u8> = row.get(1)?;
                let payload_json: String = row.get(2)?;
                Ok((row.get::<_, String>(0)?, vector_blob, payload_json))
            })?;

            let mut items = Vec::new();
            for row in rows {
                let (id, vector_blob, payload_json) = row?;
                let vector: Vec<f32> = serde_json::from_slice(&vector_blob).unwrap_or_default();
                let payload: HashMap<String, serde_json::Value> =
                    serde_json::from_str(&payload_json).unwrap_or_default();
                items.push(VecStoreItem {
                    id,
                    vector,
                    payload,
                });
            }
            Ok(items)
        })
        .map_err(|e| VecStoreError::Other(e.to_string()))
    }

    async fn delete(&self, ids: &[String], collection: Option<&str>) -> Result<(), VecStoreError> {
        if ids.is_empty() {
            return Ok(());
        }

        let coll = collection.unwrap_or("default");
        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "DELETE FROM vectors WHERE id IN ({}) AND collection = ?",
            placeholders.join(",")
        );

        self.with_conn(|conn| {
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = ids
                .iter()
                .map(|s| Box::new(s.clone()) as Box<dyn rusqlite::ToSql>)
                .collect();
            params.push(Box::new(coll.to_string()));
            let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
            conn.execute(&sql, param_refs.as_slice())?;
            Ok(())
        })
        .map_err(|e| VecStoreError::Other(e.to_string()))?;

        Ok(())
    }

    async fn upsert(
        &self,
        items: &[VecStoreItem],
        collection: Option<&str>,
    ) -> Result<(), VecStoreError> {
        // SQLite's INSERT OR REPLACE handles upsert
        self.add(items, collection).await
    }
}
