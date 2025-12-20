use color_eyre::eyre::Result;
use opentelemetry::{global, trace::TracerProvider, KeyValue};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use std::env;
use std::time::Duration;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub fn setup_otlp() -> Result<(SdkTracerProvider, SdkMeterProvider)> {
    let otlp_endpoint = env::var("OTLP_ENDPOINT").expect("env var OTLP_ENDPOINT is not set");

    let tracer_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&otlp_endpoint)
        .with_protocol(Protocol::Grpc)
        .build()?;

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(tracer_exporter)
        .with_resource(Resource::builder().with_service_name("hq").build())
        .build();

    global::set_tracer_provider(tracer_provider.clone());

    let tracer = tracer_provider.tracer("hq-tracer");
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .init();

    let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(&otlp_endpoint)
        .with_protocol(Protocol::Grpc)
        .build()?;

    let reader = PeriodicReader::builder(metric_exporter)
        .with_interval(Duration::from_secs(10))
        .build();

    let meter_provider = SdkMeterProvider::builder()
        .with_resource(Resource::builder().with_service_name("hq").build())
        .with_reader(reader)
        .build();

    global::set_meter_provider(meter_provider.clone());

    info!("otel setup completed");
    Ok((tracer_provider, meter_provider))
}

pub fn on_shutdown(
    (tracer_provider, meter_provider): (SdkTracerProvider, SdkMeterProvider),
) -> Result<()> {
    tracer_provider.shutdown()?;
    meter_provider.shutdown()?;
    Ok(())
}

pub fn send_health_check_status(host_id: impl ToString, success: bool) {
    let counter = global::meter("hq")
        .u64_counter("health_check_status")
        .build();
    counter.add(
        1,
        &[
            KeyValue::new("result", if success { "success" } else { "failure" }),
            KeyValue::new("host_id", host_id.to_string()),
        ],
    );
}

pub fn send_health_check_removed(count: usize) {
    let gauge = global::meter("hq")
        .f64_gauge("health_check_removed")
        .build();
    gauge.record(count as f64, &[]);
}

pub fn send_health_check_duration(host_id: impl ToString, duration: std::time::Duration) {
    let histogram = global::meter("hq")
        .f64_histogram("health_check_duration_seconds")
        .build();
    histogram.record(
        duration.as_secs_f64(),
        &[KeyValue::new("host_id", host_id.to_string())],
    );
}

pub fn send_reaper_terminate_candidates(count: usize) {
    let gauge = global::meter("hq")
        .f64_gauge("reaper_terminate_candidates")
        .build();
    gauge.record(count as f64, &[]);
}

pub fn send_reaper_terminate_attempts() {
    let counter = global::meter("hq")
        .u64_counter("reaper_terminate_attempts")
        .build();
    counter.add(1, &[]);
}

pub fn send_sync_host_info_status(success: bool) {
    let counter = global::meter("hq")
        .u64_counter("sync_host_info_status")
        .build();
    counter.add(
        1,
        &[KeyValue::new(
            "result",
            if success { "success" } else { "failure" },
        )],
    );
}

pub fn send_dns_healthy_ips(count: usize) {
    let gauge = global::meter("hq").f64_gauge("dns_healthy_ips").build();
    gauge.record(count as f64, &[]);
}

pub fn send_dns_sync_status(success: bool) {
    let counter = global::meter("hq").u64_counter("dns_sync_status").build();
    counter.add(
        1,
        &[KeyValue::new(
            "result",
            if success { "success" } else { "failure" },
        )],
    );
}

pub struct HqHeartbeat;

impl HqHeartbeat {
    pub fn send(self) {
        let counter = global::meter("hq").u64_counter("hq_heartbeat").build();
        counter.add(1, &[]);
    }
}
