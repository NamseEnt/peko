use color_eyre::eyre::Result;
use opentelemetry::{KeyValue, global, trace::TracerProvider};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
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

pub fn send_list_hosts_status(success: bool) {
    let counter = global::meter("hq").u64_counter("list_hosts_status").build();
    counter.add(
        1,
        &[KeyValue::new(
            "result",
            if success { "success" } else { "failure" },
        )],
    );
}

pub fn send_host_connect_attempt(host_id: impl ToString) {
    let counter = global::meter("hq")
        .u64_counter("host_connect_attempts")
        .build();
    counter.add(1, &[KeyValue::new("host_id", host_id.to_string())]);
}

pub fn send_host_connect_status(host_id: impl ToString, success: bool) {
    let counter = global::meter("hq")
        .u64_counter("host_connect_status")
        .build();
    counter.add(
        1,
        &[
            KeyValue::new("result", if success { "success" } else { "failure" }),
            KeyValue::new("host_id", host_id.to_string()),
        ],
    );
}

pub fn send_host_connect_duration(host_id: impl ToString, duration: std::time::Duration) {
    let histogram = global::meter("hq")
        .f64_histogram("host_connect_duration_seconds")
        .build();
    histogram.record(
        duration.as_secs_f64(),
        &[KeyValue::new("host_id", host_id.to_string())],
    );
}

pub fn send_pong_received(host_id: impl ToString) {
    let counter = global::meter("hq").u64_counter("pong_received").build();
    counter.add(1, &[KeyValue::new("host_id", host_id.to_string())]);
}

pub fn send_deployment_updates_sent(host_id: impl ToString, update_count: usize) {
    let counter = global::meter("hq")
        .u64_counter("deployment_updates_sent")
        .build();
    counter.add(
        update_count as u64,
        &[KeyValue::new("host_id", host_id.to_string())],
    );
}

pub fn send_deployment_updates_status(success: bool) {
    let counter = global::meter("hq")
        .u64_counter("deployment_updates_status")
        .build();
    counter.add(
        1,
        &[KeyValue::new(
            "result",
            if success { "success" } else { "failure" },
        )],
    );
}

pub fn send_ping_sent_status(success: bool) {
    let counter = global::meter("hq").u64_counter("ping_sent_status").build();
    counter.add(
        1,
        &[KeyValue::new(
            "result",
            if success { "success" } else { "failure" },
        )],
    );
}

pub fn send_reaper_removed_count(count: usize) {
    let gauge = global::meter("hq")
        .f64_gauge("reaper_removed_count")
        .build();
    gauge.record(count as f64, &[]);
}

pub fn send_active_connections(count: usize) {
    let gauge = global::meter("hq").f64_gauge("active_connections").build();
    gauge.record(count as f64, &[]);
}

pub fn send_known_hosts(count: usize) {
    let gauge = global::meter("hq").f64_gauge("known_hosts").build();
    gauge.record(count as f64, &[]);
}

pub fn send_dead_hosts(count: usize) {
    let gauge = global::meter("hq").f64_gauge("dead_hosts").build();
    gauge.record(count as f64, &[]);
}

pub fn send_deployment_db_sync_status(success: bool) {
    let counter = global::meter("hq")
        .u64_counter("deployment_db_sync_status")
        .build();
    counter.add(
        1,
        &[KeyValue::new(
            "result",
            if success { "success" } else { "failure" },
        )],
    );
}

pub fn send_deployment_db_cached_count(count: usize) {
    let gauge = global::meter("hq")
        .f64_gauge("deployment_db_cached_count")
        .build();
    gauge.record(count as f64, &[]);
}

pub fn send_deployment_db_new_loaded(count: usize) {
    let counter = global::meter("hq")
        .u64_counter("deployment_db_new_loaded")
        .build();
    counter.add(count as u64, &[]);
}

pub fn send_deployment_db_connect_status(success: bool) {
    let counter = global::meter("hq")
        .u64_counter("deployment_db_connect_status")
        .build();
    counter.add(
        1,
        &[KeyValue::new(
            "result",
            if success { "success" } else { "failure" },
        )],
    );
}

pub fn send_deployment_db_fetch_status(success: bool) {
    let counter = global::meter("hq")
        .u64_counter("deployment_db_fetch_status")
        .build();
    counter.add(
        1,
        &[KeyValue::new(
            "result",
            if success { "success" } else { "failure" },
        )],
    );
}
