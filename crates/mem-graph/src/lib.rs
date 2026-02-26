//! Graph store trait and in-memory implementation.

mod entity_knowledge_graph;
mod memory;
mod store;

#[cfg(feature = "sqlite")]
mod sqlite;

pub use entity_knowledge_graph::{
    EntityKgError, EntityKgSnapshot, EntityKgStats, EntityKnowledgeGraph,
};
pub use mem_types::{
    GraphDirection, GraphNeighbor, GraphPath, GraphStoreError, MemoryEdge, MemoryNode, VecSearchHit,
};
pub use memory::InMemoryGraphStore;
pub use store::GraphStore;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteGraphStore;
