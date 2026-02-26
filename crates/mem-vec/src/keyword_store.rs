//! In-memory keyword (BM25-like) store for hybrid search.

use mem_types::{KeywordSearchHit, KeywordStore, KeywordStoreError};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Per-user index: term -> doc_id -> term frequency; doc_id -> document length.
struct UserIndex {
    /// term -> (doc_id -> count)
    term_doc_tf: HashMap<String, HashMap<String, u32>>,
    /// doc_id -> document length (number of terms)
    doc_length: HashMap<String, u32>,
}

impl UserIndex {
    fn new() -> Self {
        Self {
            term_doc_tf: HashMap::new(),
            doc_length: HashMap::new(),
        }
    }

    fn index_doc(&mut self, doc_id: &str, text: &str) {
        let terms = tokenize(text);
        let len = terms.len() as u32;
        self.doc_length.insert(doc_id.to_string(), len);

        let mut term_counts: HashMap<String, u32> = HashMap::new();
        for t in &terms {
            *term_counts.entry(t.clone()).or_insert(0) += 1;
        }
        for (term, count) in term_counts {
            self.term_doc_tf
                .entry(term)
                .or_default()
                .insert(doc_id.to_string(), count);
        }
    }

    fn remove_doc(&mut self, doc_id: &str) {
        self.doc_length.remove(doc_id);
        for postings in self.term_doc_tf.values_mut() {
            postings.remove(doc_id);
        }
    }

    fn search(&self, query: &str, top_k: usize, min_score: f64) -> Vec<KeywordSearchHit> {
        let query_terms = tokenize(query);
        if query_terms.is_empty() {
            return vec![];
        }

        let n = self.doc_length.len() as f64;
        if n == 0.0 {
            return vec![];
        }
        let avg_len = self.doc_length.values().sum::<u32>() as f64 / n;

        let k1 = 1.2;
        let b = 0.75;

        let mut doc_scores: HashMap<String, f64> = HashMap::new();

        for term in &query_terms {
            let postings = match self.term_doc_tf.get(term) {
                Some(p) => p,
                None => continue,
            };
            let df = postings.len() as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();

            for (doc_id, &tf) in postings {
                let len = self.doc_length.get(doc_id).copied().unwrap_or(0) as f64;
                let norm =
                    (tf as f64 * (k1 + 1.0)) / (tf as f64 + k1 * (1.0 - b + b * len / avg_len));
                *doc_scores.entry(doc_id.clone()).or_insert(0.0) += idf * norm;
            }
        }

        let mut hits: Vec<KeywordSearchHit> = doc_scores
            .into_iter()
            .filter(|(_, s)| *s >= min_score)
            .map(|(id, score)| KeywordSearchHit { id, score })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(top_k);
        hits
    }
}

/// In-memory keyword store (BM25-like scoring) scoped by user/cube.
pub struct InMemoryKeywordStore {
    /// user_name -> index
    by_user: Arc<RwLock<HashMap<String, UserIndex>>>,
}

impl InMemoryKeywordStore {
    pub fn new() -> Self {
        Self {
            by_user: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn user_key(user_name: Option<&str>) -> String {
        user_name.unwrap_or("").to_string()
    }
}

impl Default for InMemoryKeywordStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl KeywordStore for InMemoryKeywordStore {
    async fn index(
        &self,
        memory_id: &str,
        text: &str,
        user_name: Option<&str>,
    ) -> Result<(), KeywordStoreError> {
        let key = Self::user_key(user_name);
        let mut guard = self.by_user.write().await;
        let idx = guard.entry(key).or_insert_with(UserIndex::new);
        idx.index_doc(memory_id, text);
        Ok(())
    }

    async fn remove(
        &self,
        memory_id: &str,
        user_name: Option<&str>,
    ) -> Result<(), KeywordStoreError> {
        let key = Self::user_key(user_name);
        let mut guard = self.by_user.write().await;
        if let Some(idx) = guard.get_mut(&key) {
            idx.remove_doc(memory_id);
        }
        Ok(())
    }

    async fn search(
        &self,
        query: &str,
        top_k: usize,
        user_name: Option<&str>,
        _filter: Option<&HashMap<String, serde_json::Value>>,
    ) -> Result<Vec<KeywordSearchHit>, KeywordStoreError> {
        let key = Self::user_key(user_name);
        let guard = self.by_user.read().await;
        let hits = guard
            .get(&key)
            .map(|idx| idx.search(query, top_k, 0.0))
            .unwrap_or_default();
        Ok(hits)
    }
}
