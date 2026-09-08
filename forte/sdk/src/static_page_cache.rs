//! Invalidates the edge copy of a page served under `#[cache_static]`.
//!
//! A static page is otherwise replaced only by a deploy: the edge holds it for
//! a year and the deploy-time purge drops the whole project at once. An app
//! whose page content comes from data it writes at runtime — a published
//! episode, an edited title — calls [`purge`] at the end of that write, and the
//! next visitor gets a freshly rendered page.
//!
//! Paths are route paths as a visitor requests them (`/episode/1`), not object
//! keys, and they are resolved against the project's own domain. Query strings
//! and fragments are rejected, because a static page is keyed by path alone.
//!
//! Returns once the invalidation is queued, not once the edge is consistent.

use crate::http::{Body, Client, Method, Request, StatusCode};

#[derive(Debug)]
pub enum Error {
    /// A path is not one a static page can be served at.
    UnusablePath(String),
    /// The project has spent its invalidation budget for this hour.
    RateLimited,
    Transport(String),
    UnexpectedStatus {
        status: u16,
        message: String,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnusablePath(message) => write!(formatter, "unusable page path: {message}"),
            Self::RateLimited => {
                write!(formatter, "page cache purge refused: hourly limit reached")
            }
            Self::Transport(message) => write!(formatter, "page cache purge failed: {message}"),
            Self::UnexpectedStatus { status, message } => {
                write!(
                    formatter,
                    "page cache purge returned status {status}: {message}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Queues an edge invalidation for each path.
///
/// Panics if `FN0_STATIC_PAGE_CACHE_URL` is not set — the fn0 runtime always
/// injects it, so its absence is a deployment fault rather than something an
/// app can recover from.
pub async fn purge(paths: &[&str]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let endpoint =
        std::env::var("FN0_STATIC_PAGE_CACHE_URL").expect("FN0_STATIC_PAGE_CACHE_URL must be set");
    let body = serde_json::json!({ "paths": paths }).to_string();

    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("{}/purge", endpoint.trim_end_matches('/')))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .map_err(|error| Error::Transport(error.to_string()))?;

    let response = Client::new()
        .send(request)
        .await
        .map_err(|error| Error::Transport(error.to_string()))?;

    let status = response.status();
    if status == StatusCode::ACCEPTED {
        return Ok(());
    }

    let body = response
        .into_body()
        .bytes()
        .await
        .map_err(|error| Error::Transport(error.to_string()))?;
    let message = String::from_utf8_lossy(&body).chars().take(512).collect();
    match status {
        StatusCode::BAD_REQUEST => Err(Error::UnusablePath(message)),
        StatusCode::TOO_MANY_REQUESTS => Err(Error::RateLimited),
        _ => Err(Error::UnexpectedStatus {
            status: status.as_u16(),
            message,
        }),
    }
}
