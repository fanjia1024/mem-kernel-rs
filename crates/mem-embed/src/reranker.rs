//! HTTP reranker client for hybrid search.

use mem_types::{RerankError, RerankHit, Reranker};
use serde::Deserialize;

/// Reranker that calls an HTTP API (e.g. Cohere, Jina, or custom).
/// Expects POST to model_url with JSON body:
///   { "query": "<query>", "documents": ["<doc1>", "<doc2>", ...] }
/// and response:
///   { "results": [ { "index": 0, "relevance_score": 0.95 }, ... ] }
/// or { "results": [ { "index": 0, "score": 0.95 }, ... ] }
pub struct HttpReranker {
    client: reqwest::Client,
    model_url: String,
    api_key: Option<String>,
}

impl HttpReranker {
    pub fn new(model_url: String, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            model_url,
            api_key,
        }
    }
}

#[derive(Deserialize)]
struct RerankResultItem {
    index: u32,
    #[serde(alias = "relevance_score", alias = "score")]
    score: f64,
}

#[derive(Deserialize)]
struct RerankApiResponse {
    results: Vec<RerankResultItem>,
}

#[async_trait::async_trait]
impl Reranker for HttpReranker {
    async fn rerank(
        &self,
        query: &str,
        doc_ids: &[String],
        documents: &[String],
        top_k: u32,
    ) -> Result<Vec<RerankHit>, RerankError> {
        if doc_ids.is_empty() || documents.is_empty() || doc_ids.len() != documents.len() {
            return Ok(vec![]);
        }

        let body = serde_json::json!({
            "query": query,
            "documents": documents,
        });

        let mut req = self.client.post(&self.model_url).json(&body);
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let res = req
            .send()
            .await
            .map_err(|e| RerankError::Other(e.to_string()))?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(RerankError::Other(format!(
                "rerank API error {}: {}",
                status, text
            )));
        }

        let parsed: RerankApiResponse = res
            .json()
            .await
            .map_err(|e| RerankError::Other(e.to_string()))?;

        let top_k = top_k as usize;
        let hits: Vec<RerankHit> = parsed
            .results
            .into_iter()
            .take(top_k)
            .filter_map(|r| {
                let idx = r.index as usize;
                doc_ids.get(idx).map(|id| RerankHit {
                    memory_id: id.clone(),
                    score: r.score,
                })
            })
            .collect();
        Ok(hits)
    }
}
