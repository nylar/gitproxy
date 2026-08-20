use tokio::signal;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use gitproxy::{config::Config, error::Result};

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

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    tracing::info!(target: "Serving app", port = config.port, root_dir = config.root_dir.to_str());
    axum::serve(listener, gitproxy::app(&config).await?)
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
