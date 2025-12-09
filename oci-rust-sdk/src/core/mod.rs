pub mod error;
pub mod retry;
pub mod region;
pub mod auth;
pub mod client;

pub use error::{OciError, Result};
pub use client::OciClient;
pub use retry::{Retrier, RetryConfiguration};

use std::time::Duration;

pub struct ClientConfig<A: auth::AuthProvider + 'static> {
    pub auth_provider: A,
    pub region: region::Region,
    pub timeout: Duration,
}
