use connectrpc::{
    ConnectError, Interceptor, Next,
    interceptor::{UnaryRequest, UnaryResponse},
};

pub struct Logging;

#[connectrpc::async_trait]
impl Interceptor for Logging {
    async fn intercept_unary(
        &self,
        req: UnaryRequest,
        next: Next<'_>,
    ) -> Result<UnaryResponse, ConnectError> {
        let path = req.ctx.path().unwrap_or("<unknown>").to_owned();
        let started = std::time::Instant::now();
        let resp = next.run(req).await;
        tracing::info!(rpc = %path, elapsed = ?started.elapsed(), ok = resp.is_ok());
        resp
    }
}
