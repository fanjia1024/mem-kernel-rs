//! Vector store trait with in-memory and Qdrant implementations.

mod memory_vec;
mod store;

#[cfg(feature = "qdrant")]
mod qdrant_store;

pub use mem_types::{VecSearchHit, VecStoreError, VecStoreItem};
pub use memory_vec::InMemoryVecStore;
#[cfg(feature = "qdrant")]
pub use qdrant_store::QdrantVecStore;
pub use store::VecStore;
