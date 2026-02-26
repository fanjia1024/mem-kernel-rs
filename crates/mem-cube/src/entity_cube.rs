//! Entity-aware MemCube wrapper.
//!
//! Wraps NaiveMemCube to add entity extraction and entity knowledge graph management.

use super::naive::NaiveMemCube;
use crate::MemCubeError;
use async_trait::async_trait;
use mem_embed::EntityExtractor;
use mem_graph::{EntityKnowledgeGraph, GraphStore};
use mem_types::*;
use mem_vec::VecStore;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Entity-aware MemCube configuration.
#[derive(Debug, Clone)]
pub struct EntityCubeConfig {
    /// Enable automatic entity extraction on add.
    pub enable_extraction: bool,
    /// Enable automatic relation extraction.
    pub extract_relations: bool,
    /// Extraction config passed to extractor.
    pub extraction_config: ExtractionConfig,
    /// Maximum number of entities per memory.
    pub max_entities_per_memory: usize,
    /// Whether to run extraction asynchronously.
    pub async_extraction: bool,
}

impl Default for EntityCubeConfig {
    fn default() -> Self {
        Self {
            enable_extraction: true,
            extract_relations: true,
            extraction_config: ExtractionConfig::default(),
            max_entities_per_memory: 50,
            async_extraction: true,
        }
    }
}

/// Entity-aware MemCube wrapper that integrates NER and Entity KG.
pub struct EntityAwareMemCube<G, V, E> {
    /// Inner naive MemCube.
    inner: NaiveMemCube<G, V, E>,
    /// Entity extractor (optional).
    extractor: Option<Arc<dyn EntityExtractor>>,
    /// Entity knowledge graph.
    entity_kg: Arc<Mutex<EntityKnowledgeGraph>>,
    /// Configuration.
    config: EntityCubeConfig,
}

impl<G, V, E> EntityAwareMemCube<G, V, E> {
    /// Create a new EntityAwareMemCube wrapping a NaiveMemCube.
    pub fn new(
        inner: NaiveMemCube<G, V, E>,
        entity_kg: EntityKnowledgeGraph,
        config: Option<EntityCubeConfig>,
    ) -> Self {
        Self {
            inner,
            extractor: None,
            entity_kg: Arc::new(Mutex::new(entity_kg)),
            config: config.unwrap_or_default(),
        }
    }

    /// Create with a custom entity extractor.
    pub fn with_extractor(
        inner: NaiveMemCube<G, V, E>,
        extractor: Arc<dyn EntityExtractor>,
        entity_kg: EntityKnowledgeGraph,
        config: Option<EntityCubeConfig>,
    ) -> Self {
        Self {
            inner,
            extractor: Some(extractor),
            entity_kg: Arc::new(Mutex::new(entity_kg)),
            config: config.unwrap_or_default(),
        }
    }

    /// Get reference to inner cube.
    pub fn inner(&self) -> &NaiveMemCube<G, V, E> {
        &self.inner
    }

    /// Get mutable reference to inner cube.
    pub fn inner_mut(&mut self) -> &mut NaiveMemCube<G, V, E> {
        &mut self.inner
    }

    /// Get entity knowledge graph.
    pub fn entity_kg(&self) -> &Arc<Mutex<EntityKnowledgeGraph>> {
        &self.entity_kg
    }

    /// Perform entity extraction and update the entity knowledge graph.
    async fn extract_and_index_entities(
        &self,
        content: &str,
        memory_id: &str,
    ) -> Result<(), MemCubeError> {
        let Some(ref extractor) = self.extractor else {
            return Ok(());
        };

        if !self.config.enable_extraction {
            return Ok(());
        }

        let result = extractor
            .extract(content, self.config.extraction_config.clone())
            .await
            .map_err(|e| MemCubeError::Other(format!("Entity extraction failed: {}", e)))?;

        let entity_kg = self.entity_kg.lock().await;

        // Limit number of entities
        let entities: Vec<_> = result
            .entities
            .into_iter()
            .take(self.config.max_entities_per_memory)
            .collect();

        // Index entities
        for entity in &entities {
            if let Err(e) = entity_kg.upsert_entity(entity, memory_id) {
                tracing::warn!(memory_id = memory_id, error = %e, "Failed to upsert entity");
            }
        }

        // Index relations if enabled
        if self.config.extract_relations {
            for relation in &result.relations {
                if let Err(e) = entity_kg.add_relation_by_name(
                    &relation.source_text,
                    &relation.target_text,
                    relation.relation_type.clone(),
                ) {
                    tracing::warn!(
                        source = %relation.source_text,
                        target = %relation.target_text,
                        error = %e,
                        "Failed to add entity relation"
                    );
                }
            }
        }

        Ok(())
    }

    /// Extract entities from a batch of memories.
    #[allow(dead_code)]
    async fn extract_batch(&self, contents: &[(String, String)]) -> Result<(), MemCubeError>
    where
        G: GraphStore + Send + Sync,
        V: VecStore + Send + Sync,
        E: mem_embed::Embedder + Send + Sync,
    {
        let Some(ref extractor) = self.extractor else {
            return Ok(());
        };

        if !self.config.enable_extraction {
            return Ok(());
        }

        let texts: Vec<String> = contents.iter().map(|(c, _)| c.clone()).collect();
        let results = extractor
            .extract_batch(&texts, self.config.extraction_config.clone())
            .await
            .map_err(|e| MemCubeError::Other(format!("Batch extraction failed: {}", e)))?;

        let entity_kg = self.entity_kg.lock().await;

        for ((_content, memory_id), result) in contents.iter().zip(results.iter()) {
            let entities: Vec<_> = result
                .entities
                .iter()
                .take(self.config.max_entities_per_memory)
                .collect();

            for entity in entities {
                if let Err(e) = entity_kg.upsert_entity(entity, memory_id) {
                    tracing::warn!(memory_id = memory_id, error = %e, "Failed to upsert entity");
                }
            }

            // Index relations
            if self.config.extract_relations {
                for relation in &result.relations {
                    if let Err(e) = entity_kg.add_relation_by_name(
                        &relation.source_text,
                        &relation.target_text,
                        relation.relation_type.clone(),
                    ) {
                        tracing::warn!(
                            source = %relation.source_text,
                            target = %relation.target_text,
                            error = %e,
                            "Failed to add entity relation"
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

#[async_trait]
impl<G, V, E> MemCube for EntityAwareMemCube<G, V, E>
where
    G: GraphStore + Send + Sync,
    V: VecStore + Send + Sync,
    E: mem_embed::Embedder + Send + Sync,
{
    async fn add_memories(&self, req: &ApiAddRequest) -> Result<MemoryResponse, MemCubeError> {
        let content = req.content_to_store().ok_or_else(|| {
            MemCubeError::Other("no messages or memory_content in request".to_string())
        })?;

        // Add memory to inner cube first so we get the real memory ID
        let response = self.inner.add_memories(req).await?;

        let memory_id = response
            .data
            .as_ref()
            .and_then(|d| d.first())
            .and_then(|o| o.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string);

        if let Some(memory_id) = memory_id {
            if self.config.async_extraction {
                let content = content.clone();
                let extractor = self.extractor.clone();
                let kg = self.entity_kg.clone();
                let config = self.config.clone();

                tokio::spawn(async move {
                    if let Some(ref extractor) = extractor {
                        if let Ok(result) = extractor.extract(&content, config.extraction_config).await
                        {
                            let kg = kg.lock().await;
                            for entity in result
                                .entities
                                .into_iter()
                                .take(config.max_entities_per_memory)
                            {
                                let _ = kg.upsert_entity(&entity, &memory_id);
                            }
                            for relation in result.relations {
                                let _ = kg.add_relation_by_name(
                                    &relation.source_text,
                                    &relation.target_text,
                                    relation.relation_type.clone(),
                                );
                            }
                        }
                    }
                });
            } else {
                self.extract_and_index_entities(&content, &memory_id)
                    .await?;
            }
        }

        Ok(response)
    }

    async fn search_memories(
        &self,
        req: &ApiSearchRequest,
    ) -> Result<SearchResponse, MemCubeError> {
        self.inner.search_memories(req).await
    }

    async fn update_memory(
        &self,
        req: &UpdateMemoryRequest,
    ) -> Result<UpdateMemoryResponse, MemCubeError> {
        // Extract entities from updated content if memory changed
        if let Some(ref memory) = req.memory {
            // Get current entities for this memory
            let entity_kg = self.entity_kg.lock().await;
            let _current_entities = entity_kg.get_entities_for_memory(&req.memory_id);

            // Re-extract entities from new content
            if let Some(ref extractor) = self.extractor {
                if let Ok(result) = extractor
                    .extract(memory, self.config.extraction_config.clone())
                    .await
                {
                    let entity_kg = self.entity_kg.lock().await;

                    // Update each entity with the new memory association
                    for entity in result.entities {
                        let normalized = entity_kg.normalize_name(&entity.text);
                        if let Some(existing_id) =
                            entity_kg.get_entity_id_by_normalized_name(&normalized)
                        {
                            let _ = entity_kg.add_memory_to_entity(&existing_id, &req.memory_id);
                        }
                    }
                }
            }
        }

        self.inner.update_memory(req).await
    }

    async fn forget_memory(
        &self,
        req: &ForgetMemoryRequest,
    ) -> Result<ForgetMemoryResponse, MemCubeError> {
        // Remove entity associations before forgetting
        let entity_kg = self.entity_kg.lock().await;
        let memory_id = req.memory_id.clone();
        let entity_ids: Vec<String> = entity_kg.get_entity_ids_for_memory(&memory_id);
        drop(entity_kg);

        // Dissociate from each entity
        for entity_id in &entity_ids {
            let entity_kg = self.entity_kg.lock().await;
            entity_kg.dissociate_from_memory(entity_id, &memory_id);
        }

        self.inner.forget_memory(req).await
    }

    async fn get_memory(&self, req: &GetMemoryRequest) -> Result<GetMemoryResponse, MemCubeError> {
        self.inner.get_memory(req).await
    }

    async fn graph_neighbors(
        &self,
        req: &GraphNeighborsRequest,
    ) -> Result<GraphNeighborsResponse, MemCubeError> {
        self.inner.graph_neighbors(req).await
    }

    async fn graph_path(&self, req: &GraphPathRequest) -> Result<GraphPathResponse, MemCubeError> {
        self.inner.graph_path(req).await
    }

    async fn graph_paths(
        &self,
        req: &GraphPathsRequest,
    ) -> Result<GraphPathsResponse, MemCubeError> {
        self.inner.graph_paths(req).await
    }
}

// ============================================================================
// Entity-specific API methods (not part of MemCube trait)
// ============================================================================

impl<G, V, E> EntityAwareMemCube<G, V, E>
where
    G: GraphStore + Send + Sync,
    V: VecStore + Send + Sync,
    E: mem_embed::Embedder + Send + Sync,
{
    /// Search for entities by name.
    pub async fn search_entities(
        &self,
        query: &str,
        entity_type: Option<EntityType>,
        limit: u32,
    ) -> Vec<Entity> {
        let entity_kg = self.entity_kg.lock().await;
        entity_kg.search_by_type_and_name(entity_type, query, limit)
    }

    /// Get entity by ID.
    pub async fn get_entity(&self, entity_id: &str) -> Option<Entity> {
        let entity_kg = self.entity_kg.lock().await;
        entity_kg.get_by_id(entity_id)
    }

    /// Get entity by name.
    pub async fn get_entity_by_name(&self, name: &str) -> Option<Entity> {
        let entity_kg = self.entity_kg.lock().await;
        entity_kg.get_by_name(name)
    }

    /// Get entities associated with a memory.
    pub async fn get_memory_entities(&self, memory_id: &str) -> Vec<Entity> {
        let entity_kg = self.entity_kg.lock().await;
        entity_kg.get_entities_for_memory(memory_id)
    }

    /// Get related entities for an entity.
    pub async fn get_entity_relations(
        &self,
        entity_id: &str,
        relation_type: Option<EntityRelationType>,
    ) -> Vec<(EntityRelationType, Entity)> {
        let entity_kg = self.entity_kg.lock().await;
        if let Some(rt) = relation_type {
            let entities = entity_kg.get_relations_by_type(entity_id, rt.clone());
            entities.into_iter().map(|e| (rt.clone(), e)).collect()
        } else {
            entity_kg
                .get_relations(entity_id)
                .into_iter()
                .flat_map(|(rt, entities)| entities.into_iter().map(move |e| (rt.clone(), e)))
                .collect()
        }
    }

    /// Get entity knowledge graph statistics.
    pub async fn entity_stats(&self) -> mem_graph::EntityKgStats {
        let entity_kg = self.entity_kg.lock().await;
        entity_kg.stats()
    }
}

// ============================================================================
// Import needed types
// ============================================================================
