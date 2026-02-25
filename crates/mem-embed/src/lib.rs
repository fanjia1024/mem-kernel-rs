//! OpenAI-compatible embedding client.

#[cfg(feature = "test-util")]
pub mod mock;
mod openai;

pub use mem_types::{Embedder, EmbedderError};
pub use openai::OpenAiEmbedder;

#[cfg(feature = "test-util")]
pub use mock::MockEmbedder;
