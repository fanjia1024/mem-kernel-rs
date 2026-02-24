//! In-memory vector store (brute-force KNN).

use mem_types::{VecSearchHit, VecStore, VecStoreError, VecStoreItem};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| (*x as f64) * (*y as f64)).sum();
    let na: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

/// In-memory VecStore: stores items in a map, search by brute-force cosine similarity.
pub struct InMemoryVecStore {
    /// collection name -> id -> item
    store: Arc<RwLock<HashMap<String, HashMap<String, VecStoreItem>>>>,
    default_collection: String,
}

impl InMemoryVecStore {
    pub fn new(default_collection: Option<&str>) -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
            default_collection: default_collection
                .unwrap_or("memos_memories")
                .to_string(),
        }
    }

    fn coll(&self, collection: Option<&str>) -> String {
        collection
            .unwrap_or(&self.default_collection)
            .to_string()
    }
}

#[async_trait::async_trait]
impl VecStore for InMemoryVecStore {
    async fn add(
        &self,
        items: &[VecStoreItem],
        collection: Option<&str>,
    ) -> Result<(), VecStoreError> {
        let coll = self.coll(collection);
        let mut guard = self.store.write().await;
        let map = guard.entry(coll).or_default();
        for item in items {
            map.insert(item.id.clone(), item.clone());
        }
        Ok(())
    }

    async fn search(
        &self,
        query_vector: &[f32],
        top_k: usize,
        filter: Option<&HashMap<String, serde_json::Value>>,
        collection: Option<&str>,
    ) -> Result<Vec<VecSearchHit>, VecStoreError> {
        let coll = self.coll(collection);
        let guard = self.store.read().await;
        let map = guard.get(&coll).map(|m| m.values().cloned().collect::<Vec<_>>());
        let items = map.unwrap_or_default();
        let mut candidates: Vec<(VecStoreItem, f64)> = items
            .into_iter()
            .filter(|i| {
                if let Some(f) = filter {
                    for (k, v) in f.iter() {
                        if let Some(pv) = i.payload.get(k) {
                            if pv != v {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }
                }
                true
            })
            .map(|i| {
                let score = cosine_similarity(query_vector, &i.vector);
                (i, score)
            })
            .collect();
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let hits = candidates
            .into_iter()
            .take(top_k)
            .map(|(i, score)| VecSearchHit {
                id: i.id,
                score,
            })
            .collect();
        Ok(hits)
    }

    async fn get_by_ids(
        &self,
        ids: &[String],
        collection: Option<&str>,
    ) -> Result<Vec<VecStoreItem>, VecStoreError> {
        let coll = self.coll(collection);
        let guard = self.store.read().await;
        let map = guard.get(&coll);
        let mut out = Vec::new();
        if let Some(m) = map {
            for id in ids {
                if let Some(item) = m.get(id) {
                    out.push(item.clone());
                }
            }
        }
        Ok(out)
    }

    async fn delete(&self, ids: &[String], collection: Option<&str>) -> Result<(), VecStoreError> {
        let coll = self.coll(collection);
        let mut guard = self.store.write().await;
        if let Some(m) = guard.get_mut(&coll) {
            for id in ids {
                m.remove(id);
            }
        }
        Ok(())
    }
}
