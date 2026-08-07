use std::sync::Arc;

use axum::Router;
use connectrpc::Router as ConnectRouter;
use tokio::signal;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use gitproxy::{config::Config, error::Result, interceptor::Logging, server::Server};

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

    let router = ConnectRouter::new().add_service(Arc::new(Server::new(&config)));

    let app = Router::new()
        .route("/health", axum::routing::get(|| async { "OK" }))
        .layer(TraceLayer::new_for_http())
        .fallback_service(router.into_axum_service().with_interceptor(Logging));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    tracing::info!(target: "Serving app", port = config.port, root_dir = config.root_dir.to_str());
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
