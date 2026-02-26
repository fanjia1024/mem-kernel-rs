//! OpenAI-compatible embedding client.

mod entity_extractor;
#[cfg(feature = "test-util")]
pub mod mock;
mod openai;
mod openai_entity_extractor;
mod reranker;

pub use entity_extractor::{
    CachedExtractor, CompositeExtractor, EntityExtractor, ExtractionOutput, ExtractorError,
};
pub use mem_types::ExtractionConfig;
pub use mem_types::{Embedder, EmbedderError};
pub use openai::OpenAiEmbedder;
pub use openai_entity_extractor::{OpenAiEntityExtractor, OpenAiExtractorConfig};
pub use reranker::HttpReranker;

#[cfg(feature = "test-util")]
pub use mock::MockEmbedder;
