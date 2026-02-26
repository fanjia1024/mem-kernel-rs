//! Entity extraction trait and implementations.
//!
//! Provides a unified interface for extracting named entities from text.

use async_trait::async_trait;
use mem_types::{EntityType, ExtractionConfig, ExtractionResult};

/// Result of extracting entities from a single text.
pub struct ExtractionOutput {
    /// The extraction result.
    pub result: ExtractionResult,
    /// The original text.
    pub text: String,
    /// Optional memory ID to associate.
    pub memory_id: Option<String>,
}

/// Errors that can occur during entity extraction.
#[derive(Debug, thiserror::Error)]
pub enum ExtractorError {
    #[error("API error: {0}")]
    ApiError(String),

    #[error("Invalid response format: {0}")]
    InvalidResponse(String),

    #[error("Rate limit exceeded")]
    RateLimited,

    #[error("Model not available: {0}")]
    ModelNotAvailable(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Other error: {0}")]
    Other(String),
}

/// Trait for entity extraction implementations.
#[async_trait]
pub trait EntityExtractor: Send + Sync {
    /// Extract entities from a single text.
    async fn extract(
        &self,
        text: &str,
        config: ExtractionConfig,
    ) -> Result<ExtractionResult, ExtractorError>;

    /// Extract entities from multiple texts.
    async fn extract_batch(
        &self,
        texts: &[String],
        config: ExtractionConfig,
    ) -> Result<Vec<ExtractionResult>, ExtractorError>;

    /// Get the list of entity types this extractor supports.
    fn supported_types(&self) -> Vec<EntityType>;

    /// Get the name/identifier of this extractor.
    fn name(&self) -> &str;

    /// Check if this extractor requires an API key.
    fn requires_api_key(&self) -> bool;

    /// Get the default API endpoint URL (if applicable).
    fn default_endpoint(&self) -> Option<&str> {
        None
    }
}

/// A wrapper that adds caching to any EntityExtractor.
pub struct CachedExtractor<E: EntityExtractor> {
    inner: E,
    cache: std::sync::Arc<std::sync::Mutex<lru::LruCache<String, ExtractionResult>>>,
    cache_size: usize,
}

impl<E: EntityExtractor> CachedExtractor<E> {
    /// Create a new cached extractor with the specified cache size.
    pub fn new(inner: E, cache_size: usize) -> Self {
        Self {
            inner,
            cache: std::sync::Arc::new(std::sync::Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(cache_size).unwrap(),
            ))),
            cache_size,
        }
    }
}

#[async_trait]
impl<E: EntityExtractor> EntityExtractor for CachedExtractor<E> {
    async fn extract(
        &self,
        text: &str,
        config: ExtractionConfig,
    ) -> Result<ExtractionResult, ExtractorError> {
        let key = format!("{:x}", md5::compute(text));

        // Check cache first
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(cached) = cache.get(&key) {
                return Ok(cached.clone());
            }
        }

        let result = self.inner.extract(text, config).await?;

        // Update cache
        {
            let mut cache = self.cache.lock().unwrap();
            if cache.len() < self.cache_size {
                cache.put(key, result.clone());
            }
        }

        Ok(result)
    }

    async fn extract_batch(
        &self,
        texts: &[String],
        config: ExtractionConfig,
    ) -> Result<Vec<ExtractionResult>, ExtractorError> {
        // Try to get from cache first
        let mut results = Vec::with_capacity(texts.len());
        let mut missing_indices = Vec::new();
        let mut missing_texts = Vec::new();

        for (i, text) in texts.iter().enumerate() {
            let key = format!("{:x}", md5::compute(text));
            {
                let mut cache = self.cache.lock().unwrap();
                if let Some(cached) = cache.get(&key) {
                    results.push(cached.clone());
                } else {
                    missing_indices.push(i);
                    missing_texts.push(text.clone());
                }
            }
        }

        // Fetch missing results
        if !missing_texts.is_empty() {
            let batch_results = self.inner.extract_batch(&missing_texts, config).await?;

            for (batch_idx, result) in batch_results.into_iter().enumerate() {
                let original_idx = missing_indices[batch_idx];
                // Extend results array
                while results.len() <= original_idx {
                    results.push(result.clone());
                }
                results[original_idx] = result.clone();

                // Add to cache
                let key = format!("{:x}", md5::compute(&missing_texts[batch_idx]));
                {
                    let mut cache = self.cache.lock().unwrap();
                    if cache.len() < self.cache_size {
                        cache.put(key, result);
                    }
                }
            }
        }

        Ok(results)
    }

    fn supported_types(&self) -> Vec<EntityType> {
        self.inner.supported_types()
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn requires_api_key(&self) -> bool {
        self.inner.requires_api_key()
    }
}

/// A composite extractor that tries multiple extractors in sequence.
pub struct CompositeExtractor {
    extractors: Vec<Box<dyn EntityExtractor>>,
}

impl CompositeExtractor {
    /// Create a new composite extractor.
    pub fn new(extractors: Vec<Box<dyn EntityExtractor>>) -> Self {
        Self { extractors }
    }

    /// Add an extractor to the composite.
    pub fn add_extractor(&mut self, extractor: Box<dyn EntityExtractor>) {
        self.extractors.push(extractor);
    }
}

#[async_trait]
impl EntityExtractor for CompositeExtractor {
    async fn extract(
        &self,
        text: &str,
        config: ExtractionConfig,
    ) -> Result<ExtractionResult, ExtractorError> {
        // Try each extractor until one succeeds
        for extractor in &self.extractors {
            match extractor.extract(text, config.clone()).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    tracing::warn!(
                        extractor = extractor.name(),
                        error = %e,
                        "Extractor failed, trying next"
                    );
                    continue;
                }
            }
        }
        Err(ExtractorError::Other("All extractors failed".to_string()))
    }

    async fn extract_batch(
        &self,
        texts: &[String],
        config: ExtractionConfig,
    ) -> Result<Vec<ExtractionResult>, ExtractorError> {
        // Use the first successful extractor for batch
        for extractor in &self.extractors {
            match extractor.extract_batch(texts, config.clone()).await {
                Ok(results) => return Ok(results),
                Err(e) => {
                    tracing::warn!(
                        extractor = extractor.name(),
                        error = %e,
                        "Batch extraction failed, trying next"
                    );
                    continue;
                }
            }
        }
        Err(ExtractorError::Other(
            "All batch extractors failed".to_string(),
        ))
    }

    fn supported_types(&self) -> Vec<EntityType> {
        // Union of all supported types
        let mut types = Vec::new();
        for extractor in &self.extractors {
            for t in extractor.supported_types() {
                if !types.contains(&t) {
                    types.push(t);
                }
            }
        }
        types
    }

    fn name(&self) -> &str {
        "composite"
    }

    fn requires_api_key(&self) -> bool {
        self.extractors.iter().any(|e| e.requires_api_key())
    }
}

// ============================================================================
// Utility functions
// ============================================================================

/// Apply deduplication to extraction results based on text and type.
pub fn deduplicate_entities(entities: &mut Vec<mem_types::ExtractedEntity>) {
    let mut seen = std::collections::HashMap::new();
    entities.retain(|e| {
        let key = (e.normalized_text.clone(), e.entity_type.clone());
        if let Some(existing_confidence) = seen.get(&key) {
            // Keep the higher confidence one
            if e.confidence > *existing_confidence {
                seen.insert(key, e.confidence);
                true
            } else {
                false
            }
        } else {
            seen.insert(key, e.confidence);
            true
        }
    });
}

/// Filter entities by minimum confidence.
pub fn filter_by_confidence(entities: &mut Vec<mem_types::ExtractedEntity>, min_confidence: f64) {
    entities.retain(|e| e.confidence >= min_confidence);
}

/// Filter entities to specific types.
pub fn filter_by_types(
    entities: &mut Vec<mem_types::ExtractedEntity>,
    target_types: Option<&[EntityType]>,
) {
    if let Some(types) = target_types {
        entities.retain(|e| types.contains(&e.entity_type));
    }
}
