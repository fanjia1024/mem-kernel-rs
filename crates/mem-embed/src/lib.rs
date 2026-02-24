//! OpenAI-compatible embedding client.

mod openai;
pub use openai::OpenAiEmbedder;
pub use mem_types::{Embedder, EmbedderError};
