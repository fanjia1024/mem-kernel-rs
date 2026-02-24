//! Core types and traits for MemOS-compatible memory API.
//!
//! Request/response DTOs align with MemOS `product_models.py` for JSON compatibility.

mod dto;
mod traits;

pub use dto::*;
pub use traits::*;
