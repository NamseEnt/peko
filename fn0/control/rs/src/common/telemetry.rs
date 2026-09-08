use forte_sdk::*;
use serde::Deserialize;
use std::collections::BTreeMap;

/// The attribute every log row carries to name its project. The worker runtime
/// stamps it from a trusted value the guest cannot influence (`fn0::execute`),
/// so a query pinned to it cannot be widened by anything the caller sends.
const PROJECT_ID_ATTRIBUTE: &str = "project_id";

/// The span attribute that names a trace's project. Alloy upserts it into span
/// attributes from the trusted `X-Fn0-Project-Id` header, and loggytracy reads
/// span attributes before resource attributes, so a guest-planted resource
/// attribute of the same name can neither widen a search nor forge the merged
/// attribute map (verified against the live engine, 2026-08-26).
const TRACE_PROJECT_ID_ATTRIBUTE: &str = "fn0.project_id";

pub struct TelemetryClient {
    base_url: String,
    access_client_id: String,
    access_client_secret: String,
}

/// An attribute equality filter (`attr=key=value`). Only equality, and only
/// through [`AttributeEquals::new`]: loggytracy reads the key as everything up
/// to the first operator character, so a key carrying one would assemble into a
/// different filter than the caller asked for.
pub struct AttributeEquals {
    key: String,
    value: String,
}

impl AttributeEquals {
    pub fn new(key: String, value: String) -> Result<Self, String> {
        if key.is_empty() {
            return Err("attribute filter key must not be empty".to_string());
        }
        if key.contains(['=', '!', '~', '<', '>']) {
            return Err(format!(
                "attribute filter key {key:?} must not contain any of = ! ~ < >"
            ));
        }
        Ok(Self { key, value })
    }
}

pub struct LogSearch<'a> {
    pub project_id: &'a str,
    pub start: &'a str,
    pub end: Option<&'a str>,
    pub stream: Option<&'a str>,
    pub attribute_equals: &'a [AttributeEquals],
    pub contains: &'a [String],
    pub regex: Option<&'a str>,
    pub limit: u32,
    pub direction: Direction,
}

pub struct LogHistogram<'a> {
    pub project_id: &'a str,
    pub start: &'a str,
    pub end: Option<&'a str>,
    pub stream: Option<&'a str>,
    pub attribute_equals: &'a [AttributeEquals],
    pub contains: &'a [String],
    pub regex: Option<&'a str>,
}

pub struct LogAttributeValues<'a> {
    pub project_id: &'a str,
    pub key: &'a str,
    pub start: &'a str,
    pub end: Option<&'a str>,
    pub attribute_equals: &'a [AttributeEquals],
}

#[derive(Clone, Copy)]
pub enum Direction {
    Forward,
    Backward,
}

impl Direction {
    fn as_param(self) -> &'static str {
        match self {
            Direction::Forward => "forward",
            Direction::Backward => "backward",
        }
    }
}

#[derive(Deserialize)]
pub struct LogRow {
    pub timestamp: String,
    pub line: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub struct HistogramBucket {
    pub bucket_start: String,
    pub bucket_end: String,
    pub count: u64,
}

#[derive(Deserialize)]
struct AttributeValueRow {
    value: String,
}

pub struct TraceSearch<'a> {
    pub project_id: &'a str,
    pub start: &'a str,
    pub end: Option<&'a str>,
    /// One of `unset`, `ok`, `error` — the span status intrinsic.
    pub status: Option<&'a str>,
    /// loggytracy duration syntax with a unit, e.g. `250ms` or `1.5s`.
    pub min_duration: Option<&'a str>,
    /// Anchored regex over span names: the whole name must match.
    pub name_regex: Option<&'a str>,
    pub limit: u32,
}

#[derive(Deserialize)]
pub struct TraceSummary {
    pub trace_id: String,
    pub root_service: String,
    pub root_name: String,
    pub start: String,
    pub end: String,
    pub duration: String,
    pub span_count: u64,
}

#[derive(Deserialize)]
pub struct TraceSpanRow {
    pub span_id: String,
    pub parent_span_id: String,
    pub name: String,
    pub kind: String,
    pub service: String,
    pub status: String,
    pub start: String,
    pub end: String,
    pub duration: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(default)]
    pub events: Vec<TraceSpanEvent>,
}

#[derive(Deserialize)]
pub struct TraceSpanEvent {
    pub timestamp: String,
    pub name: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

impl TelemetryClient {
    /// Reads the loggytracy query endpoint and its Cloudflare Access service
    /// token from the environment. These are operationally required — a control
    /// without them cannot serve any log view — so a missing one is fatal
    /// rather than silently degraded.
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("FN0_TELEMETRY_QUERY_URL")
                .expect("FN0_TELEMETRY_QUERY_URL not set"),
            access_client_id: std::env::var("FN0_TELEMETRY_ACCESS_CLIENT_ID")
                .expect("FN0_TELEMETRY_ACCESS_CLIENT_ID not set"),
            access_client_secret: std::env::var("FN0_TELEMETRY_ACCESS_CLIENT_SECRET")
                .expect("FN0_TELEMETRY_ACCESS_CLIENT_SECRET not set"),
        }
    }

    pub async fn search_logs(&self, query: LogSearch<'_>) -> anyhow::Result<Vec<LogRow>> {
        let mut params = QueryParams::new();
        params.push("start", query.start);
        if let Some(end) = query.end {
            params.push("end", end);
        }
        params.push(
            "attr",
            &format!("{PROJECT_ID_ATTRIBUTE}={}", query.project_id),
        );
        if let Some(stream) = query.stream {
            params.push("attr", &format!("stream={stream}"));
        }
        for filter in query.attribute_equals {
            params.push("attr", &format!("{}={}", filter.key, filter.value));
        }
        for contains in query.contains {
            params.push("contains", contains);
        }
        if let Some(regex) = query.regex {
            params.push("regex", regex);
        }
        params.push("limit", &query.limit.to_string());
        params.push("direction", query.direction.as_param());

        let body = self
            .get("/loggytracy/api/v1/logs", &params.encode())
            .await?;
        parse_ndjson(&body)
    }

    pub async fn histogram(&self, query: LogHistogram<'_>) -> anyhow::Result<Vec<HistogramBucket>> {
        let mut params = QueryParams::new();
        params.push("start", query.start);
        if let Some(end) = query.end {
            params.push("end", end);
        }
        params.push(
            "attr",
            &format!("{PROJECT_ID_ATTRIBUTE}={}", query.project_id),
        );
        if let Some(stream) = query.stream {
            params.push("attr", &format!("stream={stream}"));
        }
        for filter in query.attribute_equals {
            params.push("attr", &format!("{}={}", filter.key, filter.value));
        }
        for contains in query.contains {
            params.push("contains", contains);
        }
        if let Some(regex) = query.regex {
            params.push("regex", regex);
        }

        let body = self
            .get("/loggytracy/api/v1/logs/histogram", &params.encode())
            .await?;
        parse_ndjson(&body)
    }

    /// The values loggytracy has seen for one attribute key, project-scoped the
    /// same way every log query is. The backend samples the newest rows in the
    /// window rather than reading a catalog, so a rare or old value may be
    /// absent — good enough for autocomplete, which is this method's only
    /// caller.
    pub async fn log_attribute_values(
        &self,
        query: LogAttributeValues<'_>,
    ) -> anyhow::Result<Vec<String>> {
        let mut params = QueryParams::new();
        params.push("start", query.start);
        if let Some(end) = query.end {
            params.push("end", end);
        }
        params.push(
            "attr",
            &format!("{PROJECT_ID_ATTRIBUTE}={}", query.project_id),
        );
        for filter in query.attribute_equals {
            params.push("attr", &format!("{}={}", filter.key, filter.value));
        }

        let path = format!(
            "/loggytracy/api/v1/logs/attributes/{}/values",
            percent_encode(query.key)
        );
        let body = self.get(&path, &params.encode()).await?;
        let rows: Vec<AttributeValueRow> = parse_ndjson(&body)?;
        Ok(rows.into_iter().map(|row| row.value).collect())
    }

    pub async fn search_traces(&self, query: TraceSearch<'_>) -> anyhow::Result<Vec<TraceSummary>> {
        let mut params = QueryParams::new();
        params.push("start", query.start);
        if let Some(end) = query.end {
            params.push("end", end);
        }
        params.push(
            "attr",
            &format!("{TRACE_PROJECT_ID_ATTRIBUTE}={}", query.project_id),
        );
        if let Some(status) = query.status {
            params.push("attr", &format!("status={status}"));
        }
        if let Some(min_duration) = query.min_duration {
            params.push("attr", &format!("duration>={min_duration}"));
        }
        if let Some(name_regex) = query.name_regex {
            params.push("attr", &format!("name=~{name_regex}"));
        }
        params.push("limit", &query.limit.to_string());

        let body = self
            .get("/loggytracy/api/v1/traces", &params.encode())
            .await?;
        parse_ndjson(&body)
    }

    /// Every span of one trace that belongs to the project. The by-id endpoint
    /// takes no filters and answers with the whole tenant's spans for that id,
    /// so the project scoping happens here: only spans carrying the trusted
    /// project stamp survive, and an id whose spans all belong to someone else
    /// comes back empty — indistinguishable from an id that never existed.
    pub async fn project_trace_spans(
        &self,
        project_id: &str,
        trace_id: &str,
    ) -> anyhow::Result<Vec<TraceSpanRow>> {
        let body = self
            .get(&format!("/loggytracy/api/v1/traces/{trace_id}"), "")
            .await?;
        let spans: Vec<TraceSpanRow> = parse_ndjson(&body)?;
        Ok(spans
            .into_iter()
            .filter(|span| {
                span.attributes
                    .get(TRACE_PROJECT_ID_ATTRIBUTE)
                    .is_some_and(|value| value == project_id)
            })
            .collect())
    }

    /// loggytracy has no authentication of its own; the Cloudflare Access
    /// service token authenticates this caller at the edge, and a Transform Rule
    /// there overwrites `X-Scope-OrgID` with the tenant. So this sends the two
    /// Access headers and deliberately no tenant header of its own.
    async fn get(&self, path: &str, query: &str) -> anyhow::Result<Vec<u8>> {
        let url = if query.is_empty() {
            format!("{}{path}", self.base_url)
        } else {
            format!("{}{path}?{query}", self.base_url)
        };
        let req = http::Request::builder()
            .uri(url)
            .method("GET")
            .header("CF-Access-Client-Id", &self.access_client_id)
            .header("CF-Access-Client-Secret", &self.access_client_secret)
            .body(Vec::new())?;
        let resp = http::Client::new().send(req).await?;
        let status = resp.status().as_u16();
        let body = resp.into_body().bytes_limited(usize::MAX).await?.to_vec();
        if !(200..300).contains(&status) {
            anyhow::bail!(
                "loggytracy {path} failed (status={status}): {}",
                String::from_utf8_lossy(&body)
            );
        }
        Ok(body)
    }
}

fn parse_ndjson<T: for<'de> Deserialize<'de>>(body: &[u8]) -> anyhow::Result<Vec<T>> {
    let text = std::str::from_utf8(body)?;
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        rows.push(serde_json::from_str(line)?);
    }
    Ok(rows)
}

struct QueryParams {
    pairs: Vec<(&'static str, String)>,
}

impl QueryParams {
    fn new() -> Self {
        Self { pairs: Vec::new() }
    }

    fn push(&mut self, key: &'static str, value: &str) {
        self.pairs.push((key, value.to_string()));
    }

    fn encode(&self) -> String {
        self.pairs
            .iter()
            .map(|(key, value)| format!("{key}={}", percent_encode(value)))
            .collect::<Vec<_>>()
            .join("&")
    }
}

/// Percent-encodes a query-parameter value per RFC 3986: everything but the
/// unreserved set becomes `%XX`. The filter values come from user input, so an
/// unescaped `&` or `=` would otherwise forge extra parameters.
fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}
