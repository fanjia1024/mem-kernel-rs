//! OpenAI-compatible entity extraction client.
//!
//! Uses OpenAI or compatible APIs (like TogetherAI, Anthropic) for NER.

use super::entity_extractor::{
    deduplicate_entities, filter_by_confidence, filter_by_types, ExtractorError,
};
use crate::entity_extractor::EntityExtractor;
use mem_types::ExtractionConfig;
use mem_types::{ExtractedEntity, ExtractionResult};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// OpenAI-compatible entity extractor configuration.
#[derive(Debug, Clone)]
pub struct OpenAiExtractorConfig {
    /// API endpoint URL.
    pub api_url: String,
    /// API key.
    pub api_key: Option<String>,
    /// Model name.
    pub model: String,
    /// Temperature for generation.
    pub temperature: f64,
    /// System prompt for entity extraction.
    pub system_prompt: String,
}

impl Default for OpenAiExtractorConfig {
    fn default() -> Self {
        Self {
            api_url: "https://api.openai.com/v1/chat/completions".to_string(),
            api_key: None,
            model: "gpt-4o".to_string(),
            temperature: 0.0,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        }
    }
}

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are an expert named entity recognition system.
Extract all named entities from the given text and output JSON format.

Entity Types:
- PERSON: People names (e.g., "John Smith", "Dr. Emily Chen")
- ORGANIZATION: Companies, institutions (e.g., "Google", "Stanford University")
- LOCATION: Geographic locations (e.g., "San Francisco", "California")
- PRODUCT: Products and objects (e.g., "iPhone", "ChatGPT")
- EVENT: Events and occurrences (e.g., "World War II", "Product Launch")
- CONCEPT: Abstract concepts (e.g., "Machine Learning", "Democracy")
- EMAIL: Email addresses
- PHONE: Phone numbers
- URL: Web addresses
- DATETIME: Dates and times (e.g., "January 2024", "3:00 PM")
- NUMBER: Numeric values (e.g., "42", "1,000,000")

Relations to extract (between entities):
- part_of: X is part of Y
- works_at: X works at Y
- located_in: X is located in Y
- created_by: X was created by Y
- participated_in: X participated in Y
- related_to: X is related to Y
- owns: X owns Y

Output format:
{
    "entities": [
        {
            "text": "original text",
            "normalized_text": "lowercase version",
            "entity_type": "PERSON",
            "position": {"start": 0, "end": 10},
            "confidence": 0.95
        }
    ],
    "relations": [
        {
            "source_text": "entity A",
            "target_text": "entity B",
            "relation_type": "works_at",
            "confidence": 0.9
        }
    ],
    "summary": "optional brief summary"
}

Only output valid JSON, no additional text.
"#;

/// OpenAI-compatible entity extractor.
#[derive(Debug, Clone)]
pub struct OpenAiEntityExtractor {
    client: reqwest::Client,
    config: OpenAiExtractorConfig,
}

impl Default for OpenAiEntityExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAiEntityExtractor {
    /// Create a new extractor with default configuration.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            config: OpenAiExtractorConfig::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: OpenAiExtractorConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    /// Create from environment variables.
    pub fn from_env() -> Self {
        let api_url = std::env::var("NER_API_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());
        let api_key = std::env::var("NER_API_KEY").ok();
        let model = std::env::var("NER_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());

        let config = OpenAiExtractorConfig {
            api_url,
            api_key,
            model,
            temperature: 0.0,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        };

        Self::with_config(config)
    }

    /// Build the extraction prompt for a given text.
    fn build_prompt(&self, text: &str) -> String {
        format!(
            r#"Extract named entities from this text:

{}

Respond with JSON only.
"#,
            text
        )
    }

    /// Parse the raw JSON response from the API.
    fn parse_response(&self, response: &str) -> Result<NerApiResponse, ExtractorError> {
        // Try to extract JSON from response
        let json_str = extract_json_from_text(response).ok_or_else(|| {
            ExtractorError::InvalidResponse("No JSON found in response".to_string())
        })?;

        serde_json::from_str(json_str)
            .map_err(|e| ExtractorError::InvalidResponse(format!("JSON parse error: {}", e)))
    }
}

#[async_trait]
impl EntityExtractor for OpenAiEntityExtractor {
    async fn extract(
        &self,
        text: &str,
        config: ExtractionConfig,
    ) -> Result<ExtractionResult, ExtractorError> {
        let start_time = std::time::Instant::now();

        // Build request
        let user_prompt = self.build_prompt(text);
        let messages = vec![
            ChatMessage {
                role: "system",
                content: &self.config.system_prompt,
            },
            ChatMessage {
                role: "user",
                content: &user_prompt,
            },
        ];

        let request = ChatRequest {
            model: &self.config.model,
            messages: &messages,
            temperature: Some(self.config.temperature),
            response_format: Some(ResponseFormat::JsonObject),
        };

        // Send request
        let mut req = self.client.post(&self.config.api_url);

        if let Some(ref key) = self.config.api_key {
            req = req.bearer_auth(key);
        }

        let response = req
            .json(&request)
            .send()
            .await
            .map_err(|e| ExtractorError::ApiError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .map_err(|e| ExtractorError::ApiError(e.to_string()))?;
            return Err(ExtractorError::ApiError(format!(
                "API error {}: {}",
                status, body
            )));
        }

        let response_json: ChatResponse = response
            .json()
            .await
            .map_err(|e| ExtractorError::ApiError(e.to_string()))?;

        let content = response_json
            .choices
            .first()
            .and_then(|c| c.message.content.as_ref())
            .ok_or_else(|| ExtractorError::InvalidResponse("Empty response".to_string()))?;

        let result = self.parse_response(content)?;

        // Convert API types to mem_types
        let mut entities: Vec<ExtractedEntity> = result
            .entities
            .into_iter()
            .map(ExtractedEntity::from)
            .collect();
        let relations: Vec<mem_types::ExtractedRelation> = result
            .relations
            .into_iter()
            .map(mem_types::ExtractedRelation::from)
            .collect();

        // Apply filters
        filter_by_confidence(&mut entities, config.min_confidence);
        filter_by_types(
            &mut entities,
            config.target_types.as_deref(),
        );

        if config.enable_deduplication {
            deduplicate_entities(&mut entities);
        }

        // Calculate processing time
        let processing_time_ms = start_time.elapsed().as_millis() as u64;

        Ok(ExtractionResult {
            entities,
            relations: if config.extract_relations {
                relations
            } else {
                Vec::new()
            },
            summary: if config.generate_summary {
                result.summary
            } else {
                None
            },
            processing_time_ms,
        })
    }

    async fn extract_batch(
        &self,
        texts: &[String],
        config: ExtractionConfig,
    ) -> Result<Vec<ExtractionResult>, ExtractorError> {
        // Process in parallel batches (respecting rate limits)
        let mut results = Vec::with_capacity(texts.len());
        let batch_size = 10; // Adjust based on API limits

        for chunk in texts.chunks(batch_size) {
            let batch_futures: Vec<_> = chunk
                .iter()
                .map(|t| self.extract(t, config.clone()))
                .collect();

            let batch_results = futures::future::join_all(batch_futures).await;

            for result in batch_results {
                match result {
                    Ok(extraction) => results.push(extraction),
                    Err(e) => {
                        tracing::error!(error = %e, "Batch extraction failed for item");
                        // Return empty result for failed item
                        results.push(ExtractionResult {
                            entities: Vec::new(),
                            relations: Vec::new(),
                            summary: None,
                            processing_time_ms: 0,
                        });
                    }
                }
            }
        }

        Ok(results)
    }

    fn supported_types(&self) -> Vec<mem_types::EntityType> {
        vec![
            mem_types::EntityType::Person,
            mem_types::EntityType::Organization,
            mem_types::EntityType::Location,
            mem_types::EntityType::Product,
            mem_types::EntityType::Event,
            mem_types::EntityType::Concept,
            mem_types::EntityType::Email,
            mem_types::EntityType::Phone,
            mem_types::EntityType::Url,
            mem_types::EntityType::DateTime,
            mem_types::EntityType::Number,
        ]
    }

    fn name(&self) -> &str {
        "openai-ner"
    }

    fn requires_api_key(&self) -> bool {
        true
    }

    fn default_endpoint(&self) -> Option<&str> {
        Some(&self.config.api_url)
    }
}

// ============================================================================
// Internal Types
// ============================================================================

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage<'a>],
    temperature: Option<f64>,
    response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
enum ResponseFormat {
    JsonObject,
    JsonSchema { schema: serde_json::Value },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ChatResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

/// Internal API response format from NER.
#[derive(Debug, Deserialize)]
struct NerApiResponse {
    entities: Vec<NerApiEntity>,
    relations: Vec<NerApiRelation>,
    #[serde(default)]
    summary: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NerApiEntity {
    text: String,
    #[serde(default)]
    normalized_text: String,
    entity_type: String,
    position: Position,
    confidence: f64,
}

#[derive(Debug, Deserialize)]
struct Position {
    start: usize,
    end: usize,
}

#[derive(Debug, Deserialize)]
struct NerApiRelation {
    source_text: String,
    target_text: String,
    relation_type: String,
    confidence: f64,
}

/// Convert API entity to our type.
impl From<NerApiEntity> for ExtractedEntity {
    fn from(api: NerApiEntity) -> Self {
        let normalized_text = if api.normalized_text.is_empty() {
            api.text.trim().to_lowercase()
        } else {
            api.normalized_text
        };
        ExtractedEntity {
            text: api.text,
            normalized_text,
            entity_type: api.entity_type.parse().unwrap(),
            position: mem_types::TextPosition::new(api.position.start, api.position.end),
            confidence: api.confidence,
        }
    }
}

/// Convert API relation to our type.
impl From<NerApiRelation> for mem_types::ExtractedRelation {
    fn from(api: NerApiRelation) -> Self {
        mem_types::ExtractedRelation {
            source_text: api.source_text,
            target_text: api.target_text,
            relation_type: api.relation_type.parse().unwrap(),
            confidence: api.confidence,
        }
    }
}

/// Try to extract a JSON object from a text that might have extra text around it.
fn extract_json_from_text(text: &str) -> Option<&str> {
    // Find the first { and the last }
    let start = text.find('{')?;
    let end = text.rfind('}')?;

    if start < end {
        Some(&text[start..=end])
    } else {
        None
    }
}
