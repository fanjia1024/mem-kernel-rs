//! Graph store trait and in-memory implementation.

mod memory;
mod store;

pub use memory::InMemoryGraphStore;
pub use store::GraphStore;
pub use mem_types::{GraphStoreError, MemoryNode, VecSearchHit};
