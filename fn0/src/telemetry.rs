use opentelemetry::{KeyValue, global, trace::TracerProvider};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::time::Duration;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub fn setup_telemetry(
    otlp_endpoint: Option<String>,
) -> anyhow::Result<Option<(SdkTracerProvider, SdkMeterProvider)>> {
    let Some(endpoint) = otlp_endpoint else {
        tracing_subscriber::fmt::init();
        info!("telemetry setup completed (stdout-only mode)");
        return Ok(None);
    };
    // OTLP mode: export to remote endpoint + stdout
    let tracer_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&endpoint)
        .with_protocol(Protocol::Grpc)
        .build()?;

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(tracer_exporter)
        .with_resource(Resource::builder().with_service_name("fn0").build())
        .build();

    global::set_tracer_provider(tracer_provider.clone());

    let tracer = tracer_provider.tracer("fn0-tracer");
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .init();

    let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(&endpoint)
        .with_protocol(Protocol::Grpc)
        .build()?;

    let reader = PeriodicReader::builder(metric_exporter)
        .with_interval(Duration::from_secs(10))
        .build();

    let meter_provider = SdkMeterProvider::builder()
        .with_resource(Resource::builder().with_service_name("fn0").build())
        .with_reader(reader)
        .build();

    global::set_meter_provider(meter_provider.clone());

    info!("telemetry setup completed with OTLP endpoint: {}", endpoint);
    Ok(Some((tracer_provider, meter_provider)))
}

pub fn shutdown_telemetry(
    providers: Option<(SdkTracerProvider, SdkMeterProvider)>,
) -> anyhow::Result<()> {
    if let Some((tracer_provider, meter_provider)) = providers {
        tracer_provider.shutdown()?;
        meter_provider.shutdown()?;
    }
    Ok(())
}

// Telemetry event functions

pub fn wasmtime_error(func: &'static str, code_id: &str, error: &str) {
    let counter = global::meter("fn0").u64_counter("wasmtime_error").build();
    counter.add(
        1,
        &[
            KeyValue::new("func", func),
            KeyValue::new("code_id", code_id.to_string()),
            KeyValue::new("error", error.to_string()),
        ],
    );
}

pub fn oneshot_drop_before_response(code_id: &str) {
    let counter = global::meter("fn0")
        .u64_counter("oneshot_drop_before_response")
        .build();
    counter.add(1, &[KeyValue::new("code_id", code_id.to_string())]);
}

pub fn proxy_returns_error_code(code_id: &str, error_code: &str) {
    let counter = global::meter("fn0")
        .u64_counter("proxy_returns_error_code")
        .build();
    counter.add(
        1,
        &[
            KeyValue::new("code_id", code_id.to_string()),
            KeyValue::new("error_code", error_code.to_string()),
        ],
    );
}

pub fn request_task_join_error(code_id: &str, error: &str) {
    let counter = global::meter("fn0")
        .u64_counter("request_task_join_error")
        .build();
    counter.add(
        1,
        &[
            KeyValue::new("code_id", code_id.to_string()),
            KeyValue::new("error", error.to_string()),
        ],
    );
}

pub fn cpu_time(code_id: &str, cpu_time: Duration) {
    let histogram = global::meter("fn0")
        .f64_histogram("cpu_time_seconds")
        .build();
    histogram.record(
        cpu_time.as_secs_f64(),
        &[KeyValue::new("code_id", code_id.to_string())],
    );
}

pub fn cpu_timeout(code_id: &str, cpu_time: Duration) {
    let counter = global::meter("fn0").u64_counter("cpu_timeout").build();
    counter.add(1, &[KeyValue::new("code_id", code_id.to_string())]);

    // Also record the timeout duration
    let histogram = global::meter("fn0")
        .f64_histogram("cpu_timeout_seconds")
        .build();
    histogram.record(
        cpu_time.as_secs_f64(),
        &[KeyValue::new("code_id", code_id.to_string())],
    );
}

pub fn trapped(code_id: &str, trap: &str) {
    let counter = global::meter("fn0").u64_counter("trapped").build();
    counter.add(
        1,
        &[
            KeyValue::new("code_id", code_id.to_string()),
            KeyValue::new("trap", trap.to_string()),
        ],
    );
}

pub fn canceled_unexpectedly(code_id: &str, error: &str) {
    let counter = global::meter("fn0")
        .u64_counter("canceled_unexpectedly")
        .build();
    counter.add(
        1,
        &[
            KeyValue::new("code_id", code_id.to_string()),
            KeyValue::new("error", error.to_string()),
        ],
    );
}

pub fn create_instance(code_id: &str) {
    let counter = global::meter("fn0").u64_counter("create_instance").build();
    counter.add(1, &[KeyValue::new("code_id", code_id.to_string())]);
}

pub fn proxy_cache_error(code_id: &str, error: &str) {
    let counter = global::meter("fn0")
        .u64_counter("proxy_cache_error")
        .build();
    counter.add(
        1,
        &[
            KeyValue::new("code_id", code_id.to_string()),
            KeyValue::new("error", error.to_string()),
        ],
    );
}

pub fn code_id_parse_error() {
    let counter = global::meter("fn0")
        .u64_counter("code_id_parse_error")
        .build();
    counter.add(1, &[]);
}
