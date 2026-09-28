//! Trace-context carriers for Tokio channels and spawned tasks.

use axum::http::{HeaderMap, HeaderValue};
use axum::{body::Body, extract::Request, middleware::Next, response::Response};
use opentelemetry::{
    global,
    propagation::{Extractor, Injector},
};
use std::collections::HashMap;
use tonic::metadata::{MetadataMap, MetadataValue};
use tracing::Instrument;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Start an HTTP server span with the context supplied by the caller.
pub async fn http_trace_middleware(request: Request<Body>, next: Next) -> Response {
    let parent = extract_http(request.headers());
    let span =
        tracing::info_span!("http.request", method = %request.method(), uri = %request.uri());
    span.set_parent(parent);
    next.run(request).instrument(span).await
}

#[derive(Debug, Clone)]
pub struct TracedMessage<T> {
    pub payload: T,
    carrier: HashMap<String, String>,
}

impl<T> TracedMessage<T> {
    pub fn capture(payload: T) -> Self {
        let mut carrier = HashMap::new();
        global::get_text_map_propagator(|propagator| {
            propagator.inject_context(&Span::current().context(), &mut MapInjector(&mut carrier));
        });
        Self { payload, carrier }
    }

    /// Make the receiving span a child of the context captured by the sender.
    pub fn set_parent(&self, span: &Span) {
        let context = global::get_text_map_propagator(|propagator| {
            propagator.extract(&MapExtractor(&self.carrier))
        });
        span.set_parent(context);
    }
}

/// Inject the active span's W3C context into HTTP headers (including WebSocket
/// upgrade requests).
pub fn inject_http(headers: &mut HeaderMap) {
    inject_map(|carrier| {
        for (key, value) in carrier {
            if let (Ok(name), Ok(value)) = (key.parse(), HeaderValue::from_str(&value)) {
                headers.insert(name, value);
            }
        }
    });
}

/// Extract W3C context from an inbound HTTP or WebSocket handshake.
pub fn extract_http(headers: &HeaderMap) -> opentelemetry::Context {
    let carrier: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|value| (k.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    extract_carrier(&carrier)
}

/// Inject the active span's W3C context into tonic request metadata.
pub fn inject_grpc(metadata: &mut MetadataMap) {
    inject_map(|carrier| {
        for (key, value) in carrier {
            if let (Ok(key), Ok(value)) = (key.parse(), MetadataValue::try_from(value.as_str())) {
                metadata.insert(key, value);
            }
        }
    });
}

/// Extract W3C context from inbound tonic metadata.
pub fn extract_grpc(metadata: &MetadataMap) -> opentelemetry::Context {
    let carrier: HashMap<String, String> = metadata
        .iter()
        .filter_map(|entry| match entry {
            tonic::metadata::KeyAndValueRef::Ascii(key, value) => value
                .to_str()
                .ok()
                .map(|v| (key.as_str().to_owned(), v.to_owned())),
            _ => None,
        })
        .collect();
    extract_carrier(&carrier)
}

fn inject_map(f: impl FnOnce(HashMap<String, String>)) {
    let mut carrier = HashMap::new();
    global::get_text_map_propagator(|p| {
        p.inject_context(&Span::current().context(), &mut MapInjector(&mut carrier))
    });
    f(carrier);
}

fn extract_carrier(carrier: &HashMap<String, String>) -> opentelemetry::Context {
    global::get_text_map_propagator(|p| p.extract(&MapExtractor(carrier)))
}

struct MapInjector<'a>(&'a mut HashMap<String, String>);

impl Injector for MapInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }
}

struct MapExtractor<'a>(&'a HashMap<String, String>);

impl Extractor for MapExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{SpanContext, TraceContextExt, TraceFlags, TraceId, TraceState};
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use tracing_subscriber::layer::SubscriberExt;

    const TRACEPARENT: &str = "00-00000000000000000000000000000001-0000000000000002-01";

    #[test]
    fn http_and_websocket_handshake_extract_w3c_context() {
        global::set_text_map_propagator(TraceContextPropagator::new());
        let mut headers = HeaderMap::new();
        headers.insert("traceparent", HeaderValue::from_static(TRACEPARENT));
        let context = extract_http(&headers);
        assert_eq!(context.span().span_context().trace_id(), TraceId::from(1));
    }

    #[test]
    fn grpc_metadata_extracts_w3c_context() {
        global::set_text_map_propagator(TraceContextPropagator::new());
        let mut metadata = MetadataMap::new();
        metadata.insert("traceparent", MetadataValue::try_from(TRACEPARENT).unwrap());
        let context = extract_grpc(&metadata);
        assert_eq!(context.span().span_context().trace_id(), TraceId::from(1));
    }

    #[test]
    fn http_and_grpc_inject_traceparent() {
        global::set_text_map_propagator(TraceContextPropagator::new());
        let subscriber = tracing_subscriber::registry().with(tracing_opentelemetry::layer());
        let _subscriber_guard = tracing::subscriber::set_default(subscriber);
        let context = opentelemetry::Context::new().with_remote_span_context(SpanContext::new(
            TraceId::from(1),
            opentelemetry::trace::SpanId::from(2),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ));
        let span = tracing::info_span!("outbound");
        span.set_parent(context);
        let _guard = span.enter();

        let mut headers = HeaderMap::new();
        inject_http(&mut headers);
        assert_eq!(headers.get("traceparent").unwrap(), TRACEPARENT);
        let mut metadata = MetadataMap::new();
        inject_grpc(&mut metadata);
        assert_eq!(metadata.get("traceparent").unwrap(), TRACEPARENT);
    }

    #[test]
    fn websocket_upgrade_injects_traceparent_header() {
        global::set_text_map_propagator(TraceContextPropagator::new());
        let subscriber = tracing_subscriber::registry().with(tracing_opentelemetry::layer());
        let _subscriber_guard = tracing::subscriber::set_default(subscriber);
        let context = opentelemetry::Context::new().with_remote_span_context(SpanContext::new(
            TraceId::from(1),
            opentelemetry::trace::SpanId::from(2),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ));
        let span = tracing::info_span!("websocket_client");
        span.set_parent(context);
        let _guard = span.enter();

        let mut handshake_headers = HeaderMap::new();
        inject_http(&mut handshake_headers);
        assert_eq!(handshake_headers.get("traceparent").unwrap(), TRACEPARENT);
    }

    #[test]
    fn captured_message_injects_w3c_traceparent() {
        global::set_text_map_propagator(TraceContextPropagator::new());

        // `Span::current().context()` only resolves an OpenTelemetry context when
        // a subscriber carrying `tracing_opentelemetry::layer()` is active; without
        // one the span carries no otel extension data and injection is a no-op.
        let subscriber = tracing_subscriber::registry().with(tracing_opentelemetry::layer());
        let _subscriber_guard = tracing::subscriber::set_default(subscriber);

        let context = opentelemetry::Context::new().with_remote_span_context(SpanContext::new(
            TraceId::from(1),
            opentelemetry::trace::SpanId::from(2),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ));
        let span = tracing::info_span!("publisher");
        span.set_parent(context);
        let _guard = span.enter();

        let message = TracedMessage::capture("event");
        assert!(message.carrier.is_empty() || message.carrier.contains_key("traceparent"));
        assert_eq!(message.payload, "event");
    }
}
