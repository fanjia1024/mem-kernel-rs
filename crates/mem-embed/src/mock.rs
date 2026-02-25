//! Mock embedder for tests: deterministic vectors, no network.

use mem_types::{Embedder, EmbedderError};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

const DIM: usize = 1536;

/// Mock embedder that returns deterministic unit-length-ish vectors from text hash.
pub struct MockEmbedder;

impl MockEmbedder {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MockEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Embedder for MockEmbedder {
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedderError> {
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            let mut hasher = DefaultHasher::new();
            text.hash(&mut hasher);
            let h = hasher.finish();
            let mut v = Vec::with_capacity(DIM);
            for i in 0..DIM {
                let x = ((h.wrapping_add(i as u64)).wrapping_mul(0x9e3779b97f4a7c15) >> 32) as f32
                    / u32::MAX as f32;
                v.push(x * 2.0 - 1.0);
            }
            let norm: f64 = v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
            if norm > 0.0 {
                for x in &mut v {
                    *x = (*x as f64 / norm) as f32;
                }
            }
            out.push(v);
        }
        Ok(out)
    }
}
