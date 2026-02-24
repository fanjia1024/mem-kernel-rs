//! OpenAI-compatible embedding client.

mod openai;
#[cfg(feature = "test-util")]
pub mod mock;

pub use openai::OpenAiEmbedder;
pub use mem_types::{Embedder, EmbedderError};

#[cfg(feature = "test-util")]
pub use mock::MockEmbedder;
