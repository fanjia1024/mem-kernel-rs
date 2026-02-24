//! Request and response DTOs compatible with MemOS product API.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Single chat message (user/assistant).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Add-memory request (MemOS APIADDRequest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiAddRequest {
    pub user_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub writable_cube_ids: Option<Vec<String>>,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    #[serde(default = "default_async_mode")]
    pub async_mode: String,
    #[serde(default)]
    pub messages: Option<Vec<Message>>,
    #[serde(default)]
    pub memory_content: Option<String>,
    #[serde(default)]
    pub chat_history: Option<Vec<Message>>,
    #[serde(default)]
    pub custom_tags: Option<Vec<String>>,
    #[serde(default)]
    pub info: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub is_feedback: bool,
}

fn default_async_mode() -> String {
    "sync".to_string()
}

impl ApiAddRequest {
    /// Resolve cube ids to write to: writable_cube_ids or [user_id].
    pub fn writable_cube_ids(&self) -> Vec<String> {
        if let Some(ref ids) = self.writable_cube_ids {
            if !ids.is_empty() {
                return ids.clone();
            }
        }
        if let Some(ref id) = self.mem_cube_id {
            return vec![id.clone()];
        }
        vec![self.user_id.clone()]
    }

    /// Content to store: from messages or memory_content.
    pub fn content_to_store(&self) -> Option<String> {
        if let Some(ref msgs) = self.messages {
            if !msgs.is_empty() {
                let parts: Vec<String> = msgs
                    .iter()
                    .map(|m| format!("{}: {}", m.role, m.content))
                    .collect();
                return Some(parts.join("\n"));
            }
        }
        self.memory_content.clone()
    }
}

/// Search-memory request (MemOS APISearchRequest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiSearchRequest {
    pub query: String,
    pub user_id: String,
    #[serde(default)]
    pub readable_cube_ids: Option<Vec<String>>,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    #[serde(default = "default_top_k")]
    pub top_k: u32,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub relativity: f64,
    #[serde(default)]
    pub include_preference: bool,
    #[serde(default)]
    pub pref_top_k: u32,
    #[serde(default)]
    pub filter: Option<HashMap<String, serde_json::Value>>,
}

fn default_top_k() -> u32 {
    10
}

impl ApiSearchRequest {
    /// Resolve cube ids to read from: readable_cube_ids or mem_cube_id or [user_id].
    pub fn readable_cube_ids(&self) -> Vec<String> {
        if let Some(ref ids) = self.readable_cube_ids {
            if !ids.is_empty() {
                return ids.clone();
            }
        }
        if let Some(ref id) = self.mem_cube_id {
            return vec![id.clone()];
        }
        vec![self.user_id.clone()]
    }
}

/// Base response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseResponse<T> {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<T>,
}

fn default_code() -> i32 {
    200
}

/// Add-memory response (MemOS MemoryResponse).
pub type MemoryResponse = BaseResponse<Vec<serde_json::Value>>;

/// Single memory item as returned in search (id, memory, metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: String,
    pub memory: String,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// One bucket of memories (e.g. WorkingMemory, LongTermMemory).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBucket {
    pub memories: Vec<MemoryItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_nodes: Option<usize>,
}

/// Search result data: text_mem and optional pref_mem.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchResponseData {
    #[serde(default)]
    pub text_mem: Vec<MemoryBucket>,
    #[serde(default)]
    pub pref_mem: Vec<MemoryBucket>,
}

/// Search response (MemOS SearchResponse).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<SearchResponseData>,
}

/// Request to update an existing memory (partial fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMemoryRequest {
    pub memory_id: String,
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Request to forget (soft or hard delete) a memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgetMemoryRequest {
    pub memory_id: String,
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    /// If true, soft delete (mark tombstone); else hard delete.
    #[serde(default)]
    pub soft: bool,
}

/// Response for update_memory / forget_memory (same envelope as add).
pub type UpdateMemoryResponse = BaseResponse<Vec<serde_json::Value>>;
pub type ForgetMemoryResponse = BaseResponse<Vec<serde_json::Value>>;

/// Request to get a single memory by id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMemoryRequest {
    pub memory_id: String,
    pub user_id: String,
    #[serde(default)]
    pub mem_cube_id: Option<String>,
    /// If true, return memories marked tombstone (soft-deleted). Default false.
    #[serde(default)]
    pub include_deleted: bool,
}

/// Response for get_memory: optional MemoryItem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMemoryResponse {
    #[serde(default = "default_code")]
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<MemoryItem>,
}

/// Internal memory node (id, memory, metadata, optional embedding).
#[derive(Debug, Clone)]
pub struct MemoryNode {
    pub id: String,
    pub memory: String,
    pub metadata: HashMap<String, serde_json::Value>,
    pub embedding: Option<Vec<f32>>,
}

/// Scope for memory (MemOS: WorkingMemory, LongTermMemory, UserMemory).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryScope {
    WorkingMemory,
    LongTermMemory,
    UserMemory,
}

impl MemoryScope {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryScope::WorkingMemory => "WorkingMemory",
            MemoryScope::LongTermMemory => "LongTermMemory",
            MemoryScope::UserMemory => "UserMemory",
        }
    }
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
