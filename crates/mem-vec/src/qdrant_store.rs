//! Qdrant-backed vector store (requires feature "qdrant").

use mem_types::{VecSearchHit, VecStore, VecStoreError, VecStoreItem};
use qdrant_client::qdrant::{
    CreateCollectionBuilder, DeletePointsBuilder, GetPointsBuilder, PointStruct,
    SearchPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
};
use qdrant_client::Payload;
use qdrant_client::Qdrant;
use std::collections::HashMap;
use std::sync::Arc;

const DEFAULT_COLLECTION: &str = "memos_memories";
const VECTOR_SIZE: u64 = 1536;

/// Qdrant-backed implementation of VecStore.
pub struct QdrantVecStore {
    client: Arc<Qdrant>,
    collection: String,
}

impl QdrantVecStore {
    pub fn new(url: &str, collection: Option<&str>) -> Result<Self, VecStoreError> {
        let client = Qdrant::from_url(url)
            .build()
            .map_err(|e| VecStoreError::Other(e.to_string()))?;
        let collection = collection.unwrap_or(DEFAULT_COLLECTION).to_string();
        Ok(Self {
            client: Arc::new(client),
            collection,
        })
    }

    pub async fn ensure_collection(&self, vector_size: u64) -> Result<(), VecStoreError> {
        let exists = self
            .client
            .collection_exists(&self.collection)
            .await
            .map_err(|e| VecStoreError::Other(e.to_string()))?;
        if !exists {
            self.client
                .create_collection(
                    CreateCollectionBuilder::new(&self.collection).vectors_config(
                        VectorParamsBuilder::new(
                            vector_size,
                            qdrant_client::qdrant::Distance::Cosine,
                        ),
                    ),
                )
                .await
                .map_err(|e| VecStoreError::Other(e.to_string()))?;
        }
        Ok(())
    }

    fn collection(&self, override_name: Option<&str>) -> String {
        override_name.unwrap_or(&self.collection).to_string()
    }
}

#[async_trait::async_trait]
impl VecStore for QdrantVecStore {
    async fn add(
        &self,
        items: &[VecStoreItem],
        collection: Option<&str>,
    ) -> Result<(), VecStoreError> {
        let coll = self.collection(collection);
        let size = items
            .first()
            .map(|i| i.vector.len() as u64)
            .unwrap_or(VECTOR_SIZE);
        self.ensure_collection(size).await?;
        let points: Vec<PointStruct> = items
            .iter()
            .map(|i| {
                let payload_json = serde_json::Value::Object(
                    i.payload
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                );
                let payload = Payload::try_from(payload_json).unwrap_or_default();
                PointStruct::new(i.id.as_str(), i.vector.clone(), payload)
            })
            .collect();
        self.client
            .upsert_points(UpsertPointsBuilder::new(coll, points).wait(true))
            .await
            .map_err(|e| VecStoreError::Other(e.to_string()))?;
        Ok(())
    }

    async fn search(
        &self,
        query_vector: &[f32],
        top_k: usize,
        _filter: Option<&HashMap<String, serde_json::Value>>,
        collection: Option<&str>,
    ) -> Result<Vec<VecSearchHit>, VecStoreError> {
        let coll = self.collection(collection);
        let result = self
            .client
            .search_points(
                SearchPointsBuilder::new(coll, query_vector.to_vec(), top_k as u64)
                    .with_payload(true),
            )
            .await
            .map_err(|e| VecStoreError::Other(e.to_string()))?;
        let hits = result
            .result
            .into_iter()
            .map(|p| {
                let id = p
                    .id
                    .as_ref()
                    .and_then(|id| {
                        id.point_id_options.as_ref().map(|o| match o {
                            qdrant_client::qdrant::point_id::PointIdOptions::Uuid(u) => u.clone(),
                            qdrant_client::qdrant::point_id::PointIdOptions::Num(n) => {
                                n.to_string()
                            }
                        })
                    })
                    .unwrap_or_default();
                let score = p.score as f64;
                VecSearchHit { id, score }
            })
            .collect();
        Ok(hits)
    }

    async fn get_by_ids(
        &self,
        ids: &[String],
        collection: Option<&str>,
    ) -> Result<Vec<VecStoreItem>, VecStoreError> {
        let coll = self.collection(collection);
        let point_ids: Vec<qdrant_client::qdrant::PointId> = ids
            .iter()
            .map(|s| qdrant_client::qdrant::PointId::from(s.as_str()))
            .collect();
        let resp = self
            .client
            .get_points(
                GetPointsBuilder::new(coll, point_ids)
                    .with_payload(true)
                    .with_vectors(true),
            )
            .await
            .map_err(|e| VecStoreError::Other(e.to_string()))?;
        let items = resp
            .result
            .into_iter()
            .map(|p| {
                let id = p
                    .id
                    .as_ref()
                    .and_then(|id| {
                        id.point_id_options.as_ref().map(|o| match o {
                            qdrant_client::qdrant::point_id::PointIdOptions::Uuid(u) => u.clone(),
                            qdrant_client::qdrant::point_id::PointIdOptions::Num(n) => {
                                n.to_string()
                            }
                        })
                    })
                    .unwrap_or_default();
                #[allow(deprecated)]
                let vector = p
                    .vectors
                    .as_ref()
                    .and_then(|v| v.vectors_options.as_ref())
                    .and_then(|o| match o {
                        qdrant_client::qdrant::vectors_output::VectorsOptions::Vector(v) => {
                            Some(v.data.clone())
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                let payload: HashMap<String, serde_json::Value> = p
                    .payload
                    .into_iter()
                    .map(|(k, v)| {
                        let val = match v.kind.as_ref() {
                            Some(qdrant_client::qdrant::value::Kind::StringValue(s)) => {
                                serde_json::Value::String(s.clone())
                            }
                            Some(qdrant_client::qdrant::value::Kind::DoubleValue(f)) => {
                                serde_json::Number::from_f64(*f)
                                    .map(serde_json::Value::Number)
                                    .unwrap_or(serde_json::Value::Null)
                            }
                            Some(qdrant_client::qdrant::value::Kind::IntegerValue(i)) => {
                                serde_json::Value::Number(serde_json::Number::from(*i))
                            }
                            Some(qdrant_client::qdrant::value::Kind::BoolValue(b)) => {
                                serde_json::Value::Bool(*b)
                            }
                            _ => serde_json::Value::Null,
                        };
                        (k, val)
                    })
                    .collect();
                VecStoreItem {
                    id,
                    vector,
                    payload,
                }
            })
            .collect();
        Ok(items)
    }

    async fn delete(&self, ids: &[String], collection: Option<&str>) -> Result<(), VecStoreError> {
        let coll = self.collection(collection);
        let point_ids: Vec<qdrant_client::qdrant::PointId> = ids
            .iter()
            .map(|s| qdrant_client::qdrant::PointId::from(s.as_str()))
            .collect();
        self.client
            .delete_points(DeletePointsBuilder::new(coll).points(point_ids))
            .await
            .map_err(|e| VecStoreError::Other(e.to_string()))?;
        Ok(())
    }

    async fn upsert(
        &self,
        items: &[VecStoreItem],
        collection: Option<&str>,
    ) -> Result<(), VecStoreError> {
        let coll = self.collection(collection);
        let size = items
            .first()
            .map(|i| i.vector.len() as u64)
            .unwrap_or(VECTOR_SIZE);
        self.ensure_collection(size).await?;
        let points: Vec<PointStruct> = items
            .iter()
            .map(|i| {
                let payload_json = serde_json::Value::Object(
                    i.payload
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                );
                let payload = Payload::try_from(payload_json).unwrap_or_default();
                PointStruct::new(i.id.as_str(), i.vector.clone(), payload)
            })
            .collect();
        self.client
            .upsert_points(UpsertPointsBuilder::new(coll, points).wait(true))
            .await
            .map_err(|e| VecStoreError::Other(e.to_string()))?;
        Ok(())
    }
}
