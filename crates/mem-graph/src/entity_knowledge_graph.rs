//! Entity Knowledge Graph implementation.
//!
//! Provides a simple in-memory graph structure for managing entities and their relations.

use mem_types::{Entity, EntityMetadata, EntityRelationType, EntityType, ExtractedEntity};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// Entity Knowledge Graph - stores and manages entities and their relations.
#[derive(Debug, Clone, Default)]
pub struct EntityKnowledgeGraph {
    /// Core entity storage: entity_id -> Entity
    entities: Arc<RwLock<HashMap<String, Entity>>>,
    /// Name index: normalized_name -> entity_id
    name_index: Arc<RwLock<HashMap<String, String>>>,
    /// Type index: entity_type -> set of entity_ids
    type_index: Arc<RwLock<HashMap<EntityType, HashSet<String>>>>,
    /// Entity relations: source_id -> (relation_type -> set of target_ids)
    relations: Arc<RwLock<HashMap<String, HashMap<EntityRelationType, HashSet<String>>>>>,
    /// Entity to memories index: entity_id -> set of memory_ids
    memory_index: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    /// Name variants index: variant -> entity_id
    variant_index: Arc<RwLock<HashMap<String, String>>>,
}

impl EntityKnowledgeGraph {
    /// Create a new empty Entity Knowledge Graph.
    pub fn new() -> Self {
        Self {
            entities: Arc::new(RwLock::new(HashMap::new())),
            name_index: Arc::new(RwLock::new(HashMap::new())),
            type_index: Arc::new(RwLock::new(HashMap::new())),
            relations: Arc::new(RwLock::new(HashMap::new())),
            memory_index: Arc::new(RwLock::new(HashMap::new())),
            variant_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the total number of entities.
    pub fn entity_count(&self) -> usize {
        self.entities.read().unwrap().len()
    }

    /// Get the total number of relations.
    pub fn relation_count(&self) -> usize {
        self.relations
            .read()
            .unwrap()
            .values()
            .map(|rels| rels.values().map(|s| s.len()).sum::<usize>())
            .sum()
    }

    // =========================================================================
    // Entity Operations
    // =========================================================================

    /// Create or update an entity from an extraction result.
    pub fn upsert_entity(
        &self,
        extracted: &ExtractedEntity,
        memory_id: &str,
    ) -> Result<(String, bool), EntityKgError> {
        let normalized_name = self.normalize_name(&extracted.text);

        let mut entities = self.entities.write().unwrap();
        let mut name_index = self.name_index.write().unwrap();

        // Check for existing entity by name
        if let Some(existing_id) = name_index.get(&normalized_name) {
            if let Some(entity) = entities.get_mut(existing_id) {
                // Add memory ID if not already present
                if !entity.memory_ids.contains(&memory_id.to_string()) {
                    entity.memory_ids.push(memory_id.to_string());
                }

                // Add name variant if new
                if !entity.name_variants.contains(&extracted.text) {
                    entity.name_variants.push(extracted.text.clone());
                }

                // Update metadata
                entity.increment_version();
                entity.metadata.confidence =
                    (entity.metadata.confidence + extracted.confidence) / 2.0;

                // Update variant index
                let mut variant_index = self.variant_index.write().unwrap();
                variant_index.insert(extracted.text.clone(), entity.id.clone());

                return Ok((entity.id.clone(), false));
            }
        }

        // Create new entity
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let entity = Entity {
            id: id.clone(),
            name: normalized_name.clone(),
            entity_type: extracted.entity_type.clone(),
            name_variants: vec![extracted.text.clone()],
            description: None,
            memory_ids: vec![memory_id.to_string()],
            attributes: HashMap::new(),
            metadata: EntityMetadata {
                first_seen: now.clone(),
                last_updated: now,
                occurrence_count: 1,
                source_memory_id: memory_id.to_string(),
                confidence: extracted.confidence,
            },
            version: 0,
        };

        // Insert into all indices
        entities.insert(id.clone(), entity.clone());
        name_index.insert(normalized_name, id.clone());

        let mut type_index = self.type_index.write().unwrap();
        type_index
            .entry(extracted.entity_type.clone())
            .or_insert_with(HashSet::new)
            .insert(id.clone());

        let mut memory_index = self.memory_index.write().unwrap();
        memory_index
            .entry(id.clone())
            .or_insert_with(HashSet::new)
            .insert(memory_id.to_string());

        let mut variant_index = self.variant_index.write().unwrap();
        variant_index.insert(extracted.text.clone(), id.clone());

        Ok((id, true))
    }

    /// Get an entity by ID.
    pub fn get_by_id(&self, id: &str) -> Option<Entity> {
        self.entities.read().unwrap().get(id).cloned()
    }

    /// Get an entity by name (normalized).
    pub fn get_by_name(&self, name: &str) -> Option<Entity> {
        let normalized = self.normalize_name(name);
        let name_index = self.name_index.read().unwrap();
        name_index
            .get(&normalized)
            .and_then(|id| self.entities.read().unwrap().get(id).cloned())
    }

    /// Find entity by name variant (fuzzy).
    pub fn find_by_variant(&self, variant: &str) -> Option<Entity> {
        let variant_index = self.variant_index.read().unwrap();
        variant_index
            .get(variant)
            .and_then(|id| self.entities.read().unwrap().get(id).cloned())
    }

    /// Find entities by type.
    pub fn find_by_type(&self, entity_type: EntityType) -> Vec<Entity> {
        let type_index = self.type_index.read().unwrap();
        let entities = self.entities.read().unwrap();

        if let Some(ids) = type_index.get(&entity_type) {
            ids.iter()
                .filter_map(|id| entities.get(id).cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Fuzzy search entities by name.
    pub fn fuzzy_search(&self, query: &str, limit: u32) -> Vec<Entity> {
        let query_lower = query.to_lowercase();
        let entities = self.entities.read().unwrap();

        let mut results: Vec<_> = entities
            .values()
            .filter(|e| e.name.contains(&query_lower))
            .take(limit as usize)
            .cloned()
            .collect();

        // Sort by occurrence count
        results.sort_by(|a, b| {
            b.metadata
                .occurrence_count
                .cmp(&a.metadata.occurrence_count)
        });

        results
    }

    /// Search entities by type with fuzzy name matching.
    pub fn search_by_type_and_name(
        &self,
        entity_type: Option<EntityType>,
        query: &str,
        limit: u32,
    ) -> Vec<Entity> {
        let mut candidates: Vec<Entity> = if let Some(t) = entity_type {
            self.find_by_type(t)
        } else {
            self.entities.read().unwrap().values().cloned().collect()
        };

        if !query.is_empty() {
            let query_lower = query.to_lowercase();
            candidates.retain(|e| {
                e.name.contains(&query_lower)
                    || e.name_variants
                        .iter()
                        .any(|v| v.to_lowercase().contains(&query_lower))
            });
        }

        candidates.sort_by(|a, b| {
            b.metadata
                .occurrence_count
                .cmp(&a.metadata.occurrence_count)
        });
        candidates.into_iter().take(limit as usize).collect()
    }

    /// Delete an entity by ID.
    pub fn delete_entity(&self, id: &str) -> Result<(), EntityKgError> {
        let mut entities = self.entities.write().unwrap();
        if let Some(entity) = entities.remove(id) {
            // Remove from name index
            let mut name_index = self.name_index.write().unwrap();
            name_index.remove(&entity.name);

            // Remove from type index
            let mut type_index = self.type_index.write().unwrap();
            if let Some(type_set) = type_index.get_mut(&entity.entity_type) {
                type_set.remove(id);
            }

            // Remove from variant index
            let mut variant_index = self.variant_index.write().unwrap();
            for variant in &entity.name_variants {
                variant_index.remove(variant);
            }

            // Remove from memory index
            let mut memory_index = self.memory_index.write().unwrap();
            memory_index.remove(id);

            // Remove relations
            let mut relations = self.relations.write().unwrap();
            relations.remove(id);

            Ok(())
        } else {
            Err(EntityKgError::EntityNotFound(id.to_string()))
        }
    }

    /// Update entity attributes.
    pub fn update_attributes(
        &self,
        entity_id: &str,
        attributes: HashMap<String, serde_json::Value>,
    ) -> Result<(), EntityKgError> {
        let mut entities = self.entities.write().unwrap();
        if let Some(entity) = entities.get_mut(entity_id) {
            for (key, value) in attributes {
                entity.attributes.insert(key, value);
            }
            entity.increment_version();
            Ok(())
        } else {
            Err(EntityKgError::EntityNotFound(entity_id.to_string()))
        }
    }

    // =========================================================================
    // Relation Operations
    // =========================================================================

    /// Add a relation between two entities (by their normalized names).
    pub fn add_relation_by_name(
        &self,
        source_name: &str,
        target_name: &str,
        relation_type: EntityRelationType,
    ) -> Result<(), EntityKgError> {
        let name_index = self.name_index.read().unwrap();
        let source_id = name_index
            .get(&self.normalize_name(source_name))
            .ok_or_else(|| EntityKgError::EntityNotFound(source_name.to_string()))?
            .clone();

        let target_id = name_index
            .get(&self.normalize_name(target_name))
            .ok_or_else(|| EntityKgError::EntityNotFound(target_name.to_string()))?
            .clone();
        drop(name_index);

        self.add_relation(&source_id, &target_id, relation_type)
    }

    /// Add a relation between two entities (by ID).
    pub fn add_relation(
        &self,
        source_id: &str,
        target_id: &str,
        relation_type: EntityRelationType,
    ) -> Result<(), EntityKgError> {
        let entities = self.entities.read().unwrap();
        if !entities.contains_key(source_id) {
            return Err(EntityKgError::EntityNotFound(source_id.to_string()));
        }
        if !entities.contains_key(target_id) {
            return Err(EntityKgError::EntityNotFound(target_id.to_string()));
        }
        drop(entities);

        let mut relations = self.relations.write().unwrap();
        relations
            .entry(source_id.to_string())
            .or_insert_with(HashMap::new)
            .entry(relation_type)
            .or_insert_with(HashSet::new)
            .insert(target_id.to_string());

        Ok(())
    }

    /// Get all relations for an entity.
    pub fn get_relations(&self, entity_id: &str) -> Vec<(EntityRelationType, Vec<Entity>)> {
        let relations = self.relations.read().unwrap();
        let entities = self.entities.read().unwrap();

        if let Some(rel_map) = relations.get(entity_id) {
            rel_map
                .iter()
                .map(|(rel_type, target_ids)| {
                    let entities_list: Vec<Entity> = target_ids
                        .iter()
                        .filter_map(|id| entities.get(id).cloned())
                        .collect();
                    (rel_type.clone(), entities_list)
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get relations filtered by type.
    pub fn get_relations_by_type(
        &self,
        entity_id: &str,
        relation_type: EntityRelationType,
    ) -> Vec<Entity> {
        let relations = self.relations.read().unwrap();
        let entities = self.entities.read().unwrap();

        relations
            .get(entity_id)
            .and_then(|rel_map| rel_map.get(&relation_type))
            .map(|target_ids| {
                target_ids
                    .iter()
                    .filter_map(|id| entities.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Delete a relation.
    pub fn delete_relation(
        &self,
        source_id: &str,
        target_id: &str,
        relation_type: &EntityRelationType,
    ) -> Result<(), EntityKgError> {
        let mut relations = self.relations.write().unwrap();
        if let Some(rels) = relations.get_mut(source_id) {
            if let Some(targets) = rels.get_mut(relation_type) {
                targets.remove(target_id);
                Ok(())
            } else {
                Err(EntityKgError::RelationNotFound)
            }
        } else {
            Err(EntityKgError::RelationNotFound)
        }
    }

    // =========================================================================
    // Memory-Entity Association
    // =========================================================================

    /// Get all entities associated with a memory.
    pub fn get_entities_for_memory(&self, memory_id: &str) -> Vec<Entity> {
        let memory_index = self.memory_index.read().unwrap();
        let entities = self.entities.read().unwrap();

        memory_index
            .iter()
            .filter(|(_, memory_ids)| memory_ids.contains(memory_id))
            .filter_map(|(entity_id, _)| entities.get(entity_id).cloned())
            .collect()
    }

    /// Get all memory IDs associated with an entity.
    pub fn get_memory_ids_for_entity(&self, entity_id: &str) -> Vec<String> {
        let memory_index = self.memory_index.read().unwrap();
        memory_index
            .get(entity_id)
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Associate an entity with a new memory.
    pub fn associate_with_memory(&self, entity_id: &str, memory_id: &str) {
        let mut memory_index = self.memory_index.write().unwrap();
        memory_index
            .entry(entity_id.to_string())
            .or_insert_with(HashSet::new)
            .insert(memory_id.to_string());

        if let Some(mut entity) = self.entities.write().unwrap().get_mut(entity_id) {
            if !entity.memory_ids.contains(&memory_id.to_string()) {
                entity.memory_ids.push(memory_id.to_string());
                entity.metadata.occurrence_count += 1;
            }
        }
    }

    /// Remove association between entity and memory.
    pub fn dissociate_from_memory(&self, entity_id: &str, memory_id: &str) {
        let mut memory_index = self.memory_index.write().unwrap();
        if let Some(ids) = memory_index.get_mut(entity_id) {
            ids.remove(memory_id);
        }

        if let Some(mut entity) = self.entities.write().unwrap().get_mut(entity_id) {
            entity.memory_ids.retain(|id| id != memory_id);
        }
    }

    // =========================================================================
    // Statistics
    // =========================================================================

    /// Get entity statistics.
    pub fn stats(&self) -> EntityKgStats {
        let type_index = self.type_index.read().unwrap();
        let mut type_counts: HashMap<String, u32> = HashMap::new();
        for (entity_type, ids) in type_index.iter() {
            *type_counts.entry(entity_type.to_string()).or_insert(0) += ids.len() as u32;
        }

        EntityKgStats {
            total_entities: self.entity_count(),
            total_relations: self.relation_count(),
            type_counts,
        }
    }

    // =========================================================================
    // Utilities and crate-internal accessors for EntityAwareMemCube
    // =========================================================================

    /// Return entity id for a normalized name, if any.
    pub fn get_entity_id_by_normalized_name(&self, normalized: &str) -> Option<String> {
        self.name_index.read().unwrap().get(normalized).cloned()
    }

    /// Add a memory id to an entity's memory_ids list and memory_index.
    pub fn add_memory_to_entity(
        &self,
        entity_id: &str,
        memory_id: &str,
    ) -> Result<(), EntityKgError> {
        let mut entities = self.entities.write().unwrap();
        if let Some(entity) = entities.get_mut(entity_id) {
            if !entity.memory_ids.contains(&memory_id.to_string()) {
                entity.memory_ids.push(memory_id.to_string());
            }
            drop(entities);
            self.memory_index
                .write()
                .unwrap()
                .entry(entity_id.to_string())
                .or_insert_with(HashSet::new)
                .insert(memory_id.to_string());
            Ok(())
        } else {
            Err(EntityKgError::EntityNotFound(entity_id.to_string()))
        }
    }

    /// Return entity ids that have the given memory_id.
    pub fn get_entity_ids_for_memory(&self, memory_id: &str) -> Vec<String> {
        self.memory_index
            .read()
            .unwrap()
            .iter()
            .filter(|(_, ids)| ids.contains(memory_id))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Normalize an entity name for consistent indexing.
    pub fn normalize_name(&self, name: &str) -> String {
        let mut name = name.trim().to_lowercase();

        // Remove common suffixes
        let suffixes = [
            "inc.",
            "llc",
            "corp.",
            "corporation",
            "ltd.",
            "co.",
            "group",
        ];
        for suffix in &suffixes {
            if let Some(idx) = name.find(&format!(" {}", suffix)) {
                name = name[..idx].trim().to_string();
            }
        }

        // Remove extra whitespace
        name.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

// ============================================================================
// Errors
// ============================================================================

/// Errors for Entity Knowledge Graph operations.
#[derive(Debug, thiserror::Error)]
pub enum EntityKgError {
    #[error("Entity not found: {0}")]
    EntityNotFound(String),

    #[error("Relation not found")]
    RelationNotFound,

    #[error("Invalid entity ID: {0}")]
    InvalidEntityId(String),

    #[error("Duplicate entity")]
    DuplicateEntity,

    #[error("Serialization error: {0}")]
    SerializationError(String),
}

// ============================================================================
// Stats and Types
// ============================================================================

/// Statistics about the entity knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityKgStats {
    pub total_entities: usize,
    pub total_relations: usize,
    pub type_counts: HashMap<String, u32>,
}

/// Snapshot of the entity knowledge graph for serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityKgSnapshot {
    pub entities: Vec<Entity>,
    pub relations: Vec<StoredRelation>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRelation {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: EntityRelationType,
}

impl EntityKnowledgeGraph {
    /// Create a snapshot for serialization.
    pub fn snapshot(&self) -> EntityKgSnapshot {
        let entities: Vec<Entity> = self.entities.read().unwrap().values().cloned().collect();

        let mut relations = Vec::new();
        let rel_map = self.relations.read().unwrap();
        for (source_id, rel_type_map) in rel_map.iter() {
            for (rel_type, target_ids) in rel_type_map.iter() {
                for target_id in target_ids.iter() {
                    relations.push(StoredRelation {
                        source_id: source_id.clone(),
                        target_id: target_id.clone(),
                        relation_type: rel_type.clone(),
                    });
                }
            }
        }

        EntityKgSnapshot {
            entities,
            relations,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Load from a snapshot.
    pub fn load_from_snapshot(&self, snapshot: EntityKgSnapshot) -> Result<(), EntityKgError> {
        // Clear existing data
        self.entities.write().unwrap().clear();
        self.name_index.write().unwrap().clear();
        self.type_index.write().unwrap().clear();
        self.relations.write().unwrap().clear();
        self.memory_index.write().unwrap().clear();
        self.variant_index.write().unwrap().clear();

        // Load entities
        for entity in snapshot.entities {
            self.entities
                .write()
                .unwrap()
                .insert(entity.id.clone(), entity.clone());
            self.name_index
                .write()
                .unwrap()
                .insert(entity.name.clone(), entity.id.clone());
            self.type_index
                .write()
                .unwrap()
                .entry(entity.entity_type.clone())
                .or_insert_with(HashSet::new)
                .insert(entity.id.clone());

            for variant in &entity.name_variants {
                self.variant_index
                    .write()
                    .unwrap()
                    .insert(variant.clone(), entity.id.clone());
            }

            for memory_id in &entity.memory_ids {
                self.memory_index
                    .write()
                    .unwrap()
                    .entry(entity.id.clone())
                    .or_insert_with(HashSet::new)
                    .insert(memory_id.clone());
            }
        }

        // Load relations
        for rel in snapshot.relations {
            self.relations
                .write()
                .unwrap()
                .entry(rel.source_id)
                .or_insert_with(HashMap::new)
                .entry(rel.relation_type)
                .or_insert_with(HashSet::new)
                .insert(rel.target_id);
        }

        Ok(())
    }
}
