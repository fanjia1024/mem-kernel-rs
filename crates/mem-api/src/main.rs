//! MemOS REST API server: /product/add, /product/search.

use mem_api::server;
use mem_cube::NaiveMemCube;
use mem_embed::OpenAiEmbedder;
use mem_graph::InMemoryGraphStore;
use mem_vec::InMemoryVecStore;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let graph = InMemoryGraphStore::new();
    let vec_store = InMemoryVecStore::new(None);
    let embedder = OpenAiEmbedder::from_env();
    let cube: Arc<dyn mem_types::MemCube + Send + Sync> =
        Arc::new(NaiveMemCube::new(graph, vec_store, embedder));

    let app = server::router(cube);
    let addr: SocketAddr = std::env::var("MEMOS_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8001".to_string())
        .parse()?;
    tracing::info!("MemOS API listening on {}", addr);
    axum::serve(
        tokio::net::TcpListener::bind(addr).await?,
        app.into_make_service(),
    )
    .await?;
    Ok(())
}
