use crate::elastic_apm::{get_current_service_name, ApmTracingConfig};
use crate::LoggerError;
use opentelemetry::sdk::trace::{config, Sampler, Tracer};
use opentelemetry::sdk::Resource;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::{ExportConfig, Protocol, WithExportConfig};
use std::collections::HashMap;
use std::time::Duration;
use tonic::metadata::MetadataMap;
use tracing::span::EnteredSpan;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub type TornadoTraceContext = HashMap<String, String>;

pub fn get_opentelemetry_tracer(
    apm_tracing_config: &ApmTracingConfig,
) -> Result<Tracer, LoggerError> {
    let mut tonic_metadata = MetadataMap::new();
    if let Some(apm_server_api_credentials) = &apm_tracing_config.apm_server_api_credentials {
        tonic_metadata.insert(
            "authorization",
            apm_server_api_credentials.to_authorization_header_value().parse()
                .map_err(|err| LoggerError::LoggerRuntimeError {
                    message: format!("Logger - Error while constructing the authorization header for tonic client. Error: {}", err)
                })?,
        );
    };

    let export_config = ExportConfig {
        endpoint: apm_tracing_config.apm_server_url.clone(),
        protocol: Protocol::Grpc,
        timeout: Duration::from_secs(10),
    };
    opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_export_config(export_config)
                .with_metadata(tonic_metadata),
        )
        .with_trace_config(config().with_sampler(Sampler::AlwaysOn).with_resource(Resource::new(
            vec![KeyValue::new("service.name", get_current_service_name()?)],
        )))
        .install_batch(opentelemetry::runtime::Tokio)
        .map_err(|err| LoggerError::LoggerRuntimeError {
            message: format!(
                "Logger - Error while installing the OpenTelemetry Tracer. Error: {:?}",
                err
            ),
        })
}

pub fn attach_context_to_span(span: &Span, tornado_parent_context: Option<TornadoTraceContext>) {
    let parent_context = tornado_parent_context
        .map(|context| global::get_text_map_propagator(|prop| prop.extract(&context)));
    if let Some(parent_context) = parent_context {
        span.set_parent(parent_context)
    }
}

pub fn get_span_context_carrier(span: &EnteredSpan) -> TornadoTraceContext {
    let mut context_carrier = HashMap::new();

    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&span.context(), &mut context_carrier)
    });

    context_carrier
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::elastic_apm::{ApmServerApiCredentials, ApmTracingConfig};

    #[tokio::test]
    async fn should_get_opentelemetry_tracer() {
        let tracing_config = ApmTracingConfig {
            apm_output: true,
            apm_server_url: "apm.example.com".to_string(),
            apm_server_api_credentials: Some(ApmServerApiCredentials {
                id: "myid".to_string(),
                key: "mykey".to_string(),
            }),
        };
        let tracer = get_opentelemetry_tracer(&tracing_config);
        assert!(tracer.is_ok());
    }
}
