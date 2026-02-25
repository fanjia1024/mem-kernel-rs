//! HTTP client for OpenAI-compatible embedding API.

use mem_types::{Embedder, EmbedderError};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct EmbedResponse {
    data: Option<Vec<EmbedItem>>,
}

#[derive(Debug, Deserialize)]
struct EmbedItem {
    embedding: Vec<f32>,
}

/// Embedder that calls an OpenAI-compatible embedding endpoint (e.g. POST /embeddings).
pub struct OpenAiEmbedder {
    client: reqwest::Client,
    url: String,
    api_key: Option<String>,
    model: String,
}

impl OpenAiEmbedder {
    pub fn new(url: String, api_key: Option<String>, model: Option<&str>) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
            api_key,
            model: model.unwrap_or("text-embedding-3-small").to_string(),
        }
    }

    pub fn from_env() -> Self {
        let url = std::env::var("EMBED_API_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1/embeddings".to_string());
        let api_key = std::env::var("EMBED_API_KEY").ok();
        let model = std::env::var("EMBED_MODEL").ok();
        Self::new(url, api_key, model.as_deref())
    }
}

#[async_trait::async_trait]
impl Embedder for OpenAiEmbedder {
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedderError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut all = Vec::with_capacity(texts.len());
        for text in texts {
            let body = serde_json::json!({
                "input": text,
                "model": self.model
            });
            let mut req = self.client.post(&self.url).json(&body);
            if let Some(ref key) = self.api_key {
                req = req.bearer_auth(key);
            }
            let res = req
                .send()
                .await
                .map_err(|e| EmbedderError::Other(e.to_string()))?;
            let status = res.status();
            let body = res
                .text()
                .await
                .map_err(|e| EmbedderError::Other(e.to_string()))?;
            if !status.is_success() {
                return Err(EmbedderError::Other(format!(
                    "embed API error {}: {}",
                    status, body
                )));
            }
            let parsed: EmbedResponse =
                serde_json::from_str(&body).map_err(|e| EmbedderError::Other(e.to_string()))?;
            let embedding = parsed
                .data
                .and_then(|d| d.into_iter().next())
                .map(|i| i.embedding)
                .ok_or(EmbedderError::EmptyResponse)?;
            all.push(embedding);
        }
        Ok(all)
    }
}
