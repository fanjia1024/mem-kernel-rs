//! Entity types for Named Entity Recognition and Entity Knowledge Graph.
//!
//! Provides structures for extracted entities, their relations, and metadata.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Entity type enumeration covering common NER categories.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    /// People, including fictional characters.
    Person,
    /// Companies, agencies, institutions.
    Organization,
    /// Geographic locations (cities, countries, etc.).
    Location,
    /// Products, objects, devices.
    Product,
    /// Events, historical moments.
    Event,
    /// Abstract concepts, ideas.
    Concept,
    /// Date and time expressions.
    DateTime,
    /// Numerical values.
    Number,
    /// Email addresses.
    Email,
    /// Phone numbers.
    Phone,
    /// URLs and web links.
    Url,
    /// Custom/domain-specific entity type.
    Custom(String),
}

impl std::fmt::Display for EntityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntityType::Person => write!(f, "PERSON"),
            EntityType::Organization => write!(f, "ORGANIZATION"),
            EntityType::Location => write!(f, "LOCATION"),
            EntityType::Product => write!(f, "PRODUCT"),
            EntityType::Event => write!(f, "EVENT"),
            EntityType::Concept => write!(f, "CONCEPT"),
            EntityType::DateTime => write!(f, "DATETIME"),
            EntityType::Number => write!(f, "NUMBER"),
            EntityType::Email => write!(f, "EMAIL"),
            EntityType::Phone => write!(f, "PHONE"),
            EntityType::Url => write!(f, "URL"),
            EntityType::Custom(s) => write!(f, "CUSTOM:{}", s),
        }
    }
}

impl EntityType {
    /// Parse from string (case-insensitive).
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "person" | "per" => EntityType::Person,
            "organization" | "org" => EntityType::Organization,
            "location" | "loc" => EntityType::Location,
            "product" | "prod" => EntityType::Product,
            "event" | "evt" => EntityType::Event,
            "concept" | "con" => EntityType::Concept,
            "datetime" | "date" => EntityType::DateTime,
            "number" | "num" => EntityType::Number,
            "email" => EntityType::Email,
            "phone" | "tel" => EntityType::Phone,
            "url" | "link" => EntityType::Url,
            _ => EntityType::Custom(s.to_string()),
        }
    }
}

/// Text position for entity mention in source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPosition {
    /// Start character index (inclusive).
    pub start: usize,
    /// End character index (exclusive).
    pub end: usize,
}

impl TextPosition {
    /// Create a new text position.
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Calculate the length of the span.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Check if position is empty.
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// Entity metadata for tracking and quality assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityMetadata {
    /// First time this entity was seen.
    pub first_seen: String,
    /// Last time this entity was updated.
    pub last_updated: String,
    /// Number of times this entity has appeared across all memories.
    pub occurrence_count: u32,
    /// ID of the memory that first created this entity.
    pub source_memory_id: String,
    /// Confidence score from NER extraction (0.0 - 1.0).
    pub confidence: f64,
}

impl EntityMetadata {
    /// Create new metadata with current timestamp.
    pub fn new(source_memory_id: String, confidence: f64) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            first_seen: now.clone(),
            last_updated: now,
            occurrence_count: 1,
            source_memory_id,
            confidence,
        }
    }
}

/// An extracted or stored entity with its associated data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Unique identifier for this entity.
    pub id: String,
    /// Normalized/canonical name for this entity.
    pub name: String,
    /// The type of this entity.
    pub entity_type: EntityType,
    /// Alternative names/variants for fuzzy matching.
    #[serde(default)]
    pub name_variants: Vec<String>,
    /// Human-readable description or summary of this entity.
    #[serde(default)]
    pub description: Option<String>,
    /// IDs of memories associated with this entity.
    #[serde(default)]
    pub memory_ids: Vec<String>,
    /// Key-value attributes/properties of this entity.
    #[serde(default)]
    pub attributes: HashMap<String, serde_json::Value>,
    /// Additional metadata.
    pub metadata: EntityMetadata,
    /// Version number for optimistic locking.
    pub version: u32,
}

impl Entity {
    /// Create a new entity with basic fields.
    pub fn new(
        id: String,
        name: String,
        entity_type: EntityType,
        source_memory_id: String,
        confidence: f64,
    ) -> Self {
        Self {
            id,
            name,
            entity_type,
            name_variants: Vec::new(),
            description: None,
            memory_ids: vec![source_memory_id.clone()],
            attributes: HashMap::new(),
            metadata: EntityMetadata::new(source_memory_id, confidence),
            version: 0,
        }
    }

    /// Add a memory ID to this entity.
    pub fn add_memory_id(&mut self, memory_id: String) {
        if !self.memory_ids.contains(&memory_id) {
            self.memory_ids.push(memory_id);
            self.metadata.occurrence_count += 1;
        }
    }

    /// Add a name variant.
    pub fn add_variant(&mut self, variant: String) {
        if !self.name_variants.contains(&variant) {
            self.name_variants.push(variant);
        }
    }

    /// Increment version for optimistic locking.
    pub fn increment_version(&mut self) {
        self.version += 1;
        self.metadata.last_updated = chrono::Utc::now().to_rfc3339();
    }
}

/// A lightweight reference to an entity within a memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityReference {
    /// Entity ID for lookup.
    pub entity_id: String,
    /// Entity name as it appears in text.
    pub name: String,
    /// Entity type.
    pub entity_type: EntityType,
    /// Position of mention in source text.
    pub position: TextPosition,
}

/// Types of relations between entities.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityRelationType {
    /// X is a part of Y.
    PartOf,
    /// X works at Y (company/organization).
    WorksAt,
    /// X is located in Y.
    LocatedIn,
    /// X was created by Y.
    CreatedBy,
    /// X participated in Y (event).
    ParticipatedIn,
    /// X is related to Y (generic association).
    RelatedTo,
    /// X owns Y.
    Owns,
    /// X graduated from Y (educational institution).
    GraduatedFrom,
    /// X is a member of Y.
    MemberOf,
    /// X is the founder of Y.
    FoundedBy,
    /// Custom relation type.
    Custom(String),
}

impl EntityRelationType {
    /// Create a copy of this relation type.
    pub fn copy(&self) -> Self {
        self.clone()
    }
}

impl std::fmt::Display for EntityRelationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntityRelationType::PartOf => write!(f, "part_of"),
            EntityRelationType::WorksAt => write!(f, "works_at"),
            EntityRelationType::LocatedIn => write!(f, "located_in"),
            EntityRelationType::CreatedBy => write!(f, "created_by"),
            EntityRelationType::ParticipatedIn => write!(f, "participated_in"),
            EntityRelationType::RelatedTo => write!(f, "related_to"),
            EntityRelationType::Owns => write!(f, "owns"),
            EntityRelationType::GraduatedFrom => write!(f, "graduated_from"),
            EntityRelationType::MemberOf => write!(f, "member_of"),
            EntityRelationType::FoundedBy => write!(f, "founded_by"),
            EntityRelationType::Custom(s) => write!(f, "custom:{}", s),
        }
    }
}

impl EntityRelationType {
    /// Parse from string.
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "part_of" | "partof" => EntityRelationType::PartOf,
            "works_at" | "worksat" | "employer" => EntityRelationType::WorksAt,
            "located_in" | "locatedin" => EntityRelationType::LocatedIn,
            "created_by" | "createdby" => EntityRelationType::CreatedBy,
            "participated_in" | "participatedin" => EntityRelationType::ParticipatedIn,
            "related_to" | "relatedto" => EntityRelationType::RelatedTo,
            "owns" | "possesses" => EntityRelationType::Owns,
            "graduated_from" | "graduatedfrom" => EntityRelationType::GraduatedFrom,
            "member_of" | "memberof" => EntityRelationType::MemberOf,
            "founded_by" | "foundedby" => EntityRelationType::FoundedBy,
            _ => EntityRelationType::Custom(s.to_string()),
        }
    }
}

/// A relation between two entities extracted from text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedRelation {
    /// The source entity text.
    pub source_text: String,
    /// The target entity text.
    pub target_text: String,
    /// The type of relation.
    pub relation_type: EntityRelationType,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f64,
}

/// Result of entity extraction from a single text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    /// List of entities extracted from the text.
    #[serde(default)]
    pub entities: Vec<ExtractedEntity>,
    /// List of relations between entities.
    #[serde(default)]
    pub relations: Vec<ExtractedRelation>,
    /// Optional summary of the text.
    #[serde(default)]
    pub summary: Option<String>,
    /// Processing time in milliseconds.
    pub processing_time_ms: u64,
}

/// A single entity extracted from text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    /// The exact text that was recognized as an entity.
    pub text: String,
    /// The normalized/canonical form of the entity name.
    #[serde(default)]
    pub normalized_text: String,
    /// The type of entity.
    pub entity_type: EntityType,
    /// Position in source text.
    pub position: TextPosition,
    /// Confidence score from the extraction model.
    pub confidence: f64,
}

impl ExtractedEntity {
    /// Create a new extracted entity.
    pub fn new(
        text: String,
        entity_type: EntityType,
        position: TextPosition,
        confidence: f64,
    ) -> Self {
        Self {
            text: text.clone(),
            normalized_text: text.trim().to_lowercase(),
            entity_type,
            position,
            confidence,
        }
    }
}

/// Configuration for entity extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionConfig {
    /// Minimum confidence threshold (0.0 - 1.0).
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f64,
    /// Filter to specific entity types (None = all types).
    #[serde(default)]
    pub target_types: Option<Vec<EntityType>>,
    /// Whether to extract relations between entities.
    #[serde(default = "default_true")]
    pub extract_relations: bool,
    /// Whether to generate a summary.
    #[serde(default)]
    pub generate_summary: bool,
    /// Whether to enable entity deduplication.
    #[serde(default = "default_true")]
    pub enable_deduplication: bool,
    /// Whether to run extraction asynchronously.
    #[serde(default = "default_true")]
    pub async_extraction: bool,
}

fn default_min_confidence() -> f64 {
    0.7
}

fn default_true() -> bool {
    true
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            min_confidence: default_min_confidence(),
            target_types: None,
            extract_relations: true,
            generate_summary: false,
            enable_deduplication: true,
            async_extraction: true,
        }
    }
}

// ============================================================================
// Hybrid Search Types (Phase 6)
// ============================================================================

/// Search channel types for hybrid search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchChannel {
    /// Vector similarity search.
    Vector,
    /// Keyword/BM25 search.
    Keyword,
    /// Graph-based search.
    Graph,
}

/// Search mode for hybrid search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HybridSearchMode {
    /// Only vector search.
    VectorOnly,
    /// Only keyword search.
    KeywordOnly,
    /// Only graph search.
    GraphOnly,
    /// Fuse all channels (vector + keyword + graph).
    Fusion,
    /// Custom channel combination.
    Custom,
}

impl Default for HybridSearchMode {
    fn default() -> Self {
        Self::Fusion
    }
}

/// Fusion weights for combining scores from different channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FusionWeights {
    /// Weight for vector similarity score.
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,
    /// Weight for keyword/BM25 score.
    #[serde(default = "default_keyword_weight")]
    pub keyword_weight: f64,
    /// Weight for graph score.
    #[serde(default = "default_graph_weight")]
    pub graph_weight: f64,
}

fn default_vector_weight() -> f64 {
    0.6
}

fn default_keyword_weight() -> f64 {
    0.3
}

fn default_graph_weight() -> f64 {
    0.1
}

impl Default for FusionWeights {
    fn default() -> Self {
        Self {
            vector_weight: default_vector_weight(),
            keyword_weight: default_keyword_weight(),
            graph_weight: default_graph_weight(),
        }
    }
}

/// Fusion strategy for combining scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FusionStrategy {
    /// Simple weighted average.
    WeightedAverage,
    /// Reciprocal Rank Fusion.
    Rrf,
    /// Combines weighted average with rank normalization.
    Hybrid,
}

/// Configuration for keyword search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordSearchConfig {
    /// Enable keyword search channel.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Minimum BM25 score threshold.
    #[serde(default)]
    pub min_score: Option<f32>,
    /// Field weights for multi-field search.
    #[serde(default)]
    pub field_weights: Option<HashMap<String, f32>>,
    /// Fields that require exact match.
    #[serde(default)]
    pub exact_match_fields: Option<Vec<String>>,
}

impl Default for KeywordSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_score: None,
            field_weights: None,
            exact_match_fields: None,
        }
    }
}

/// Configuration for graph search channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSearchConfig {
    /// Enable graph search channel.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Entity IDs to start graph traversal from.
    #[serde(default)]
    pub entity_ids: Option<Vec<String>>,
    /// Maximum traversal depth.
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    /// Filter by relation types.
    #[serde(default)]
    pub relation_types: Option<Vec<String>>,
}

fn default_max_depth() -> u32 {
    2
}

impl Default for GraphSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            entity_ids: None,
            max_depth: default_max_depth(),
            relation_types: None,
        }
    }
}

/// Configuration for reranking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankConfig {
    /// Enable reranking.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Reranker model URL.
    #[serde(default)]
    pub model_url: Option<String>,
    /// API key for reranker service.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Number of results to return after reranking.
    #[serde(default = "default_rerank_top_k")]
    pub rerank_top_k: u32,
}

fn default_rerank_top_k() -> u32 {
    5
}

impl Default for RerankConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model_url: None,
            api_key: None,
            rerank_top_k: default_rerank_top_k(),
        }
    }
}
