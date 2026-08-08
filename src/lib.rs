use std::sync::Arc;

use axum::Router;
use connectrpc::Router as ConnectRouter;
use tower_http::trace::TraceLayer;

use crate::{config::Config, interceptor::Logging, server::Server};

pub mod config;
#[rustfmt::skip]
#[path = "gen/connect/mod.rs"]
pub mod connect;
pub mod error;
pub mod interceptor;
#[rustfmt::skip]
#[path = "gen/buffa/mod.rs"]
pub mod proto;
pub mod repository;
pub mod server;
#[rustfmt::skip]
#[path = "gen/protovalidate/mod.rs"]
pub mod validate;

pub fn app(config: &Config) -> Router {
    let router = ConnectRouter::new().add_service(Arc::new(Server::new(config)));

    Router::new()
        .route("/health", axum::routing::get(|| async { "OK" }))
        .layer(TraceLayer::new_for_http())
        .fallback_service(router.into_axum_service().with_interceptor(Logging))
}
