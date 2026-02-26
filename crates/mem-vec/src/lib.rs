//! Vector store trait with in-memory and Qdrant implementations.

mod keyword_store;
mod memory_vec;
mod store;

#[cfg(feature = "qdrant")]
mod qdrant_store;

#[cfg(feature = "sqlite")]
mod sqlite_vec;

pub use keyword_store::InMemoryKeywordStore;
pub use mem_types::{VecSearchHit, VecStoreError, VecStoreItem};
pub use memory_vec::InMemoryVecStore;
#[cfg(feature = "qdrant")]
pub use qdrant_store::QdrantVecStore;
#[cfg(feature = "sqlite")]
pub use sqlite_vec::SqliteVecStore;
pub use store::VecStore;
