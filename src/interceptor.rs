use connectrpc::{
    ConnectError, Interceptor, Next, NextStream, PayloadStream,
    interceptor::{StreamRequest, StreamResponse, UnaryRequest, UnaryResponse},
};
use futures::StreamExt;

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

    async fn intercept_streaming(
        &self,
        req: StreamRequest,
        inbound: PayloadStream,
        next: NextStream<'_>,
    ) -> Result<StreamResponse, ConnectError> {
        let path = req.ctx.path().unwrap_or("<unknown>").to_owned();
        let started = std::time::Instant::now();

        let path_clone = path.clone();
        let logged = inbound.map(move |msg| {
            tracing::info!(rpc = %path_clone, direction = "client_message", ok = msg.is_ok());
            msg
        });

        let resp = next.run(req, Box::pin(logged)).await;
        match resp {
            Ok(resp) => {
                tracing::info!(rpc = %path, status = "success", elapsed = ?started.elapsed());
                Ok(resp)
            }
            Err(e) => {
                tracing::info!(rpc = %path, status = "fail", elapsed = ?started.elapsed());
                Err(e)
            }
        }
    }
}
