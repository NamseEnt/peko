//! Reading the Cloudflare setup token off the OS clipboard for
//! `forte cloud init --setup-token-from-clipboard`.
//!
//! An AI agent (or the user) creates the token in the Cloudflare dashboard and
//! clicks its "Copy" button. Nothing types the secret into a prompt, a command
//! argument, or a file, and — when an agent drives the browser — the value
//! never has to pass through the agent's own context. This polls the clipboard
//! until a value appears that Cloudflare confirms is a live token, then
//! overwrites the clipboard so the secret does not linger there.

use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::collections::HashSet;
use std::time::{Duration, Instant};

const CLOUDFLARE_API_BASE: &str = "https://api.cloudflare.com/client/v4";
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const POLL_TIMEOUT: Duration = Duration::from_secs(900);
const HEARTBEAT_EVERY: Duration = Duration::from_secs(60);
const MINIMUM_TOKEN_LENGTH: usize = 20;
const MAXIMUM_TOKEN_LENGTH: usize = 200;
const CONSUMED_MARKER: &str = "(Cloudflare setup token consumed by forte cloud init)";

pub async fn read_setup_token_from_clipboard() -> Result<String> {
    let client = reqwest::Client::new();
    let started = Instant::now();
    let mut last_heartbeat = started;
    let mut rejected: HashSet<String> = HashSet::new();
    loop {
        if let Some(candidate) = next_clipboard_candidate(&rejected).await? {
            match verify_token(&client, &candidate).await {
                VerifyOutcome::Active => {
                    overwrite_clipboard().await;
                    return Ok(candidate);
                }
                VerifyOutcome::Rejected => {
                    rejected.insert(candidate);
                }
                VerifyOutcome::Unreachable => {}
            }
        }
        if started.elapsed() >= POLL_TIMEOUT {
            return Err(anyhow!(
                "no Cloudflare setup token appeared on the clipboard within {} seconds. \
                 Create the token, copy it, and run the command again — or omit \
                 --setup-token-from-clipboard to paste it at a prompt.",
                POLL_TIMEOUT.as_secs()
            ));
        }
        if last_heartbeat.elapsed() >= HEARTBEAT_EVERY {
            println!("  still waiting for the setup token on the clipboard...");
            last_heartbeat = Instant::now();
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn next_clipboard_candidate(rejected: &HashSet<String>) -> Result<Option<String>> {
    let contents = read_clipboard().await?;
    let trimmed = contents.trim();
    if !looks_like_a_token(trimmed) || rejected.contains(trimmed) {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

fn looks_like_a_token(candidate: &str) -> bool {
    (MINIMUM_TOKEN_LENGTH..=MAXIMUM_TOKEN_LENGTH).contains(&candidate.len())
        && candidate
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

async fn read_clipboard() -> Result<String> {
    tokio::task::spawn_blocking(|| {
        let mut clipboard = arboard::Clipboard::new().map_err(|error| {
            anyhow!(
                "could not open the clipboard ({error}). This needs a desktop session; omit \
                 --setup-token-from-clipboard to paste the token at a prompt instead."
            )
        })?;
        match clipboard.get_text() {
            Ok(text) => Ok(text),
            Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
            Err(error) => Err(anyhow!("could not read the clipboard: {error}")),
        }
    })
    .await?
}

async fn overwrite_clipboard() {
    let _ = tokio::task::spawn_blocking(|| {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(CONSUMED_MARKER);
        }
    })
    .await;
}

enum VerifyOutcome {
    Active,
    Rejected,
    Unreachable,
}

#[derive(Deserialize)]
struct VerifyEnvelope {
    success: bool,
    result: Option<VerifyResult>,
}

#[derive(Deserialize)]
struct VerifyResult {
    status: String,
}

async fn verify_token(client: &reqwest::Client, token: &str) -> VerifyOutcome {
    let Ok(response) = client
        .get(format!("{CLOUDFLARE_API_BASE}/user/tokens/verify"))
        .bearer_auth(token)
        .send()
        .await
    else {
        return VerifyOutcome::Unreachable;
    };
    if response.status().is_server_error() {
        return VerifyOutcome::Unreachable;
    }
    let Ok(text) = response.text().await else {
        return VerifyOutcome::Unreachable;
    };
    let Ok(envelope) = serde_json::from_str::<VerifyEnvelope>(&text) else {
        return VerifyOutcome::Unreachable;
    };
    match envelope.result {
        Some(result) if envelope.success && result.status == "active" => VerifyOutcome::Active,
        _ => VerifyOutcome::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::looks_like_a_token;

    #[test]
    fn accepts_a_classic_length_api_token() {
        assert!(looks_like_a_token(
            "v1_AbCd3fGh1jKlMnOpQrStUvWxYz012345_-6789"
        ));
    }

    #[test]
    fn accepts_a_prefixed_token() {
        assert!(looks_like_a_token(
            "cfut_0123456789abcdefghijklmnopqrstuvwxyz"
        ));
    }

    #[test]
    fn rejects_values_that_are_not_token_shaped() {
        assert!(!looks_like_a_token(""));
        assert!(!looks_like_a_token("short"));
        assert!(!looks_like_a_token(
            "https://dash.cloudflare.com/profile/api-tokens"
        ));
        assert!(!looks_like_a_token(
            "a token with spaces that is otherwise long enough"
        ));
        assert!(!looks_like_a_token(
            &"x".repeat(super::MAXIMUM_TOKEN_LENGTH + 1)
        ));
    }
}
