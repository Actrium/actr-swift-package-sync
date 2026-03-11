#[cfg(feature = "opentelemetry")]
use actr_protocol::{RpcEnvelope, SignalingEnvelope};
#[cfg(feature = "opentelemetry")]
use opentelemetry::{
    Context,
    propagation::{Extractor, Injector},
    trace::TraceContextExt,
};
#[cfg(feature = "opentelemetry")]
use tracing::Span;
#[cfg(feature = "opentelemetry")]
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[cfg(feature = "opentelemetry")]
pub(crate) fn inject_span_context(span: &Span, envelope: &mut SignalingEnvelope) {
    let mut injector = EnvelopeInjector(envelope);
    let context = span.context();
    let span_ref = context.span();
    let span_context = span_ref.span_context();
    if !span_context.is_valid() {
        tracing::warn!("⚠️ inject_span_context: span context is not valid, skipping injection");
        return;
    }

    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&context, &mut injector)
    });
}

#[cfg(feature = "opentelemetry")]
pub(crate) fn extract_trace_context(envelope: &SignalingEnvelope) -> Context {
    let context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&EnvelopeExtractor(envelope))
    });
    let span_ref = context.span();
    let span_context = span_ref.span_context();
    if span_context.is_valid() {
        context
    } else {
        Context::current()
    }
}

#[cfg(feature = "opentelemetry")]
struct EnvelopeExtractor<'a>(&'a SignalingEnvelope);

#[cfg(feature = "opentelemetry")]
impl<'a> Extractor for EnvelopeExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        match key {
            "traceparent" => self.0.traceparent.as_deref(),
            "tracestate" => self.0.tracestate.as_deref(),
            _ => None,
        }
    }

    fn keys(&self) -> Vec<&str> {
        vec!["traceparent", "tracestate"]
    }
}

#[cfg(feature = "opentelemetry")]
struct EnvelopeInjector<'a>(&'a mut SignalingEnvelope);

#[cfg(feature = "opentelemetry")]
impl<'a> Injector for EnvelopeInjector<'a> {
    fn set(&mut self, key: &str, value: String) {
        match key {
            "traceparent" => self.0.traceparent = Some(value),
            "tracestate" => self.0.tracestate = Some(value),
            _ => {}
        }
    }
}

// ========== RpcEnvelope tracing support ==========

#[cfg(feature = "opentelemetry")]
/// Set the given span's parent from RpcEnvelope context (or current Context if invalid).
pub(crate) fn set_parent_from_rpc_envelope(span: &Span, envelope: &RpcEnvelope) {
    let context = extract_trace_context_from_rpc(envelope);
    span.set_parent(context);
}

#[cfg(feature = "opentelemetry")]
/// Inject current span context into RpcEnvelope.
pub(crate) fn inject_span_context_to_rpc(span: &Span, envelope: &mut RpcEnvelope) {
    let mut injector = RpcEnvelopeInjector(envelope);
    let context = span.context();
    let span_ref = context.span();
    let span_context = span_ref.span_context();
    if !span_context.is_valid() {
        tracing::warn!(
            "⚠️ inject_span_context_to_rpc: span context is not valid, skipping injection"
        );
        return;
    }

    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&context, &mut injector)
    });
}

#[cfg(feature = "opentelemetry")]
/// Extract trace context from RpcEnvelope.
pub(crate) fn extract_trace_context_from_rpc(envelope: &RpcEnvelope) -> Context {
    let context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&RpcEnvelopeExtractor(envelope))
    });
    let span_ref = context.span();
    let span_context = span_ref.span_context();
    if span_context.is_valid() {
        context
    } else {
        Context::current()
    }
}

#[cfg(feature = "opentelemetry")]
struct RpcEnvelopeExtractor<'a>(&'a RpcEnvelope);

#[cfg(feature = "opentelemetry")]
impl<'a> Extractor for RpcEnvelopeExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        match key {
            "traceparent" => self.0.traceparent.as_deref(),
            "tracestate" => self.0.tracestate.as_deref(),
            _ => None,
        }
    }

    fn keys(&self) -> Vec<&str> {
        vec!["traceparent", "tracestate"]
    }
}

#[cfg(feature = "opentelemetry")]
struct RpcEnvelopeInjector<'a>(&'a mut RpcEnvelope);

#[cfg(feature = "opentelemetry")]
impl<'a> Injector for RpcEnvelopeInjector<'a> {
    fn set(&mut self, key: &str, value: String) {
        match key {
            "traceparent" => self.0.traceparent = Some(value),
            "tracestate" => self.0.tracestate = Some(value),
            _ => {}
        }
    }
}
