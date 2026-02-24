//! Lifecycle and governance types: MemoryRecord, AuditEvent (for update/forget and audit).

use serde::{Deserialize, Serialize};

/// State of a memory record in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryState {
    #[default]
    Active,
    Archived,
    Tombstone,
}

/// Full record for lifecycle (versioning, state, audit). Can be derived from MemoryNode.metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub namespace: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub state: MemoryState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Kind of auditable event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditEventKind {
    Add,
    Update,
    Forget,
    Search,
}

/// One audit event (for governance and debugging).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: String,
    pub kind: AuditEventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<String>,
    pub user_id: String,
    pub cube_id: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// Options for listing audit events (filter + pagination).
#[derive(Debug, Clone, Default)]
pub struct AuditListOptions {
    pub user_id: Option<String>,
    pub cube_id: Option<String>,
    /// ISO8601 timestamp; return events with timestamp >= since.
    pub since: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}
