//! LLM client for memory summarization and other LLM-based features (P1).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

/// LLM client error.
#[derive(Debug, thiserror::Error)]
pub enum LLMError {
    #[error("LLM error: {0}")]
    Other(String),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error: {0}")]
    Api(String),
    #[error("parse error: {0}")]
    Parse(String),
}

/// Message for LLM conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role: "system", "user", or "assistant"
    pub role: String,
    /// Message content
    pub content: String,
}

/// Request to LLM chat completion.
#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<Message>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
}

/// Response from LLM chat completion.
#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(rename = "prompt_tokens")]
    prompt_tokens: Option<u32>,
    #[serde(rename = "completion_tokens")]
    completion_tokens: Option<u32>,
    #[serde(rename = "total_tokens")]
    total_tokens: Option<u32>,
}

/// LLM client trait for text generation.
#[async_trait]
pub trait LLMClient: Send + Sync {
    /// Complete a prompt.
    async fn complete(&self, prompt: &str) -> Result<String, LLMError>;

    /// Complete with conversation messages.
    async fn complete_with_messages(&self, messages: &[Message]) -> Result<String, LLMError>;
}

/// OpenAI-compatible LLM client.
pub struct OpenAiLLMClient {
    client: reqwest::Client,
    api_url: String,
    api_key: String,
    model: String,
}

impl OpenAiLLMClient {
    /// Create a new OpenAI LLM client.
    pub fn new(
        api_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_url: api_url.into(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    /// Create from environment variables.
    pub fn from_env() -> Option<Self> {
        let api_url = std::env::var("LLM_API_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());
        let api_key = std::env::var("LLM_API_KEY").ok()?;
        let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

        Some(Self::new(api_url, api_key, model))
    }
}

impl fmt::Debug for OpenAiLLMClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiLLMClient")
            .field("api_url", &self.api_url)
            .field("model", &self.model)
            .finish()
    }
}

#[async_trait]
impl LLMClient for OpenAiLLMClient {
    async fn complete(&self, prompt: &str) -> Result<String, LLMError> {
        let messages = vec![Message {
            role: "user".to_string(),
            content: prompt.to_string(),
        }];
        self.complete_with_messages(&messages).await
    }

    async fn complete_with_messages(&self, messages: &[Message]) -> Result<String, LLMError> {
        let request = ChatCompletionRequest {
            model: self.model.clone(),
            messages: messages.to_vec(),
            max_tokens: Some(4096),
            temperature: Some(0.7),
        };

        let response = self
            .client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(LLMError::Api(format!("status: {}, body: {}", status, body)));
        }

        let completion: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| LLMError::Parse(e.to_string()))?;

        let content = completion
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| LLMError::Other("no choices returned".to_string()))?;

        Ok(content)
    }
}

/// LLM client wrapper that can use different backends.
pub enum LLMClientEnum {
    #[allow(dead_code)]
    OpenAI(OpenAiLLMClient),
}

#[async_trait]
impl LLMClient for LLMClientEnum {
    async fn complete(&self, prompt: &str) -> Result<String, LLMError> {
        match self {
            LLMClientEnum::OpenAI(client) => client.complete(prompt).await,
        }
    }

    async fn complete_with_messages(&self, messages: &[Message]) -> Result<String, LLMError> {
        match self {
            LLMClientEnum::OpenAI(client) => client.complete_with_messages(messages).await,
        }
    }
}
