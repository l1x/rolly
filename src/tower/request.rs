use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use http::{Request, Response};
use pin_project_lite::pin_project;
use tower::{Layer, Service};
use tracing::field::Empty;

use crate::trace_id::{generate_span_id, generate_trace_id, hex_encode};

/// Tower Layer that extracts `x-amz-cf-id` and wraps requests in a tracing span.
///
/// Creates an `info_span!("request", ...)` with HTTP method, URI, status code,
/// latency, CloudFront request ID, and trace/span IDs. Emits RED metric events
/// on response.
#[derive(Clone, Debug)]
pub struct CfRequestIdLayer;

impl<S> Layer<S> for CfRequestIdLayer {
    type Service = CfRequestIdService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CfRequestIdService { inner }
    }
}

#[derive(Clone, Debug)]
pub struct CfRequestIdService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for CfRequestIdService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone,
    S::Future: Send + 'static,
    S::Error: std::fmt::Display,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = CfRequestIdFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let method = req.method().to_string();
        let uri = req.uri().to_string();

        let cf_id = req
            .headers()
            .get("x-amz-cf-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let cf_display = cf_id.as_deref().unwrap_or("-").to_string();
        let trace_id = generate_trace_id(cf_id.as_deref());
        let span_id = generate_span_id();
        let trace_id_hex = hex_encode(&trace_id);
        let span_id_hex = hex_encode(&span_id);

        let span = tracing::info_span!(
            "request",
            http.method = %method,
            http.uri = %uri,
            http.status_code = Empty,
            http.latency_ms = Empty,
            cf.request_id = %cf_display,
            trace_id = %trace_id_hex,
            span_id = %span_id_hex,
        );

        let start = Instant::now();
        let future = self.inner.call(req);

        CfRequestIdFuture {
            inner: future,
            span,
            start,
            method,
            uri,
        }
    }
}

pin_project! {
    pub struct CfRequestIdFuture<F> {
        #[pin]
        inner: F,
        span: tracing::Span,
        start: Instant,
        method: String,
        uri: String,
    }
}

impl<F, ResBody, E> Future for CfRequestIdFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
    E: std::fmt::Display,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _guard = this.span.enter();

        match this.inner.poll(cx) {
            Poll::Ready(result) => {
                let latency_ms = this.start.elapsed().as_secs_f64() * 1000.0;

                match &result {
                    Ok(response) => {
                        let status = response.status().as_u16();
                        this.span.record("http.status_code", status);
                        this.span.record("http.latency_ms", latency_ms);

                        tracing::info!(
                            metric = "http.server.request.duration",
                            r#type = "histogram",
                            value = latency_ms,
                            method = %this.method,
                            route = %this.uri,
                            status = status,
                        );
                        tracing::info!(
                            metric = "http.server.request.count",
                            r#type = "counter",
                            value = 1u64,
                            method = %this.method,
                            route = %this.uri,
                            status = status,
                        );
                        if status >= 400 {
                            tracing::info!(
                                metric = "http.server.error.count",
                                r#type = "counter",
                                value = 1u64,
                                method = %this.method,
                                route = %this.uri,
                                status = status,
                            );
                        }
                    }
                    Err(_) => {
                        this.span.record("http.latency_ms", latency_ms);
                    }
                }

                Poll::Ready(result)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_request_id_layer_constructs() {
        let _layer = CfRequestIdLayer;
    }

    #[test]
    fn extract_cf_id_from_header() {
        let req = Request::builder()
            .header("x-amz-cf-id", "test-cf-id-123")
            .body(())
            .unwrap();
        let cf_id = req
            .headers()
            .get("x-amz-cf-id")
            .and_then(|v| v.to_str().ok());
        assert_eq!(cf_id, Some("test-cf-id-123"));
    }

    #[test]
    fn missing_cf_id_header() {
        let req = Request::builder().body(()).unwrap();
        let cf_id = req
            .headers()
            .get("x-amz-cf-id")
            .and_then(|v| v.to_str().ok());
        assert_eq!(cf_id, None);
    }
}
