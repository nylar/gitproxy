use std::sync::Arc;

use axum::Router;
use connectrpc::Router as ConnectRouter;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{config::Config, error::Result, interceptor::Logging, server::Server};

mod config;
#[path = "gen/connect/mod.rs"]
mod connect;
mod error;
mod interceptor;
#[path = "gen/buffa/mod.rs"]
mod proto;
mod repository;
mod server;

#[tokio::main]
async fn main() -> Result<()> {
    let config = envy::prefixed("GIT_PROXY_").from_env::<Config>()?;
    std::fs::create_dir_all(&config.root_dir)?;

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_env("GIT_PROXY_LOG_LEVEL").unwrap_or_default(),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    let router =
        ConnectRouter::new().add_service(Arc::new(Server::new(config.root_dir.to_path_buf())));

    let app = Router::new()
        .route("/health", axum::routing::get(|| async { "OK" }))
        .layer(TraceLayer::new_for_http())
        .fallback_service(router.into_axum_service().with_interceptor(Logging));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    tracing::info!(target: "Serving app", port = config.port, root_dir = config.root_dir.to_str());
    axum::serve(listener, app).await?;
    Ok(())
}
