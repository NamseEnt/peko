use crate::common::auth;
use crate::docs::*;
use forte_sdk::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Input {
    pub project_id: String,
    pub task: String,
    pub input: serde_json::Value,
}

#[derive(Serialize)]
pub enum Output {
    Ok {
        status: u16,
        content_type: Option<String>,
        body: String,
    },
    NotLoggedIn,
    NotFound,
    Forbidden,
    NotDeployed,
    UpstreamError {
        status: u16,
        content_type: Option<String>,
        body: String,
    },
    InternalError {
        reason: String,
    },
}

pub async fn handler(req: ForteRequest<'_, Input>) -> Output {
    let Some(user) = auth::bearer_user(req.headers).await else {
        return Output::NotLoggedIn;
    };

    let db = doc_db::turso();

    let project = match (ProjectDocGet {
        project_id: &req.body.project_id,
    })
    .send_with(&db)
    .await
    {
        Ok(Some(p)) => p,
        Ok(None) => return Output::NotFound,
        Err(e) => {
            tracing::error!(?e, "admin_run ProjectDocGet");
            return Output::InternalError {
                reason: format!("ProjectDocGet: {e}"),
            };
        }
    };

    if project.owner_github_id != user.github_id {
        return Output::Forbidden;
    }

    let manifest = match (WorkerManifestDocGet {}).send_with(&db).await {
        Ok(Some(m)) => m,
        Ok(None) => return Output::NotDeployed,
        Err(e) => {
            tracing::error!(?e, "admin_run WorkerManifestDocGet");
            return Output::InternalError {
                reason: format!("WorkerManifestDocGet: {e}"),
            };
        }
    };

    if !manifest
        .project_manifests
        .contains_key(&req.body.project_id)
    {
        return Output::NotDeployed;
    }

    let placeholder_url = match std::env::var("FN0_CROSS_PROJECT_INVOKE_URL") {
        Ok(u) => u,
        Err(_) => {
            return Output::InternalError {
                reason: "FN0_CROSS_PROJECT_INVOKE_URL not set".to_string(),
            };
        }
    };

    let placeholder_uri: http::Uri = match placeholder_url.parse() {
        Ok(u) => u,
        Err(e) => {
            return Output::InternalError {
                reason: format!("FN0_CROSS_PROJECT_INVOKE_URL parse: {e}"),
            };
        }
    };
    let scheme = placeholder_uri.scheme_str().unwrap_or("http");
    let Some(placeholder_host) = placeholder_uri.host() else {
        return Output::InternalError {
            reason: "FN0_CROSS_PROJECT_INVOKE_URL has no host".to_string(),
        };
    };

    let target_url = format!(
        "{scheme}://{project_id}.{placeholder_host}/__forte_admin/{task}",
        scheme = scheme,
        project_id = req.body.project_id,
        placeholder_host = placeholder_host,
        task = req.body.task,
    );

    let body_bytes = match serde_json::to_vec(&req.body.input) {
        Ok(b) => b,
        Err(e) => {
            return Output::InternalError {
                reason: format!("input serialize: {e}"),
            };
        }
    };

    let request = match http::Request::builder()
        .method("POST")
        .uri(&target_url)
        .header("x-fn0-admin", "true")
        .header("content-type", "application/json")
        .body(body_bytes)
    {
        Ok(r) => r,
        Err(e) => {
            return Output::InternalError {
                reason: format!("build request: {e}"),
            };
        }
    };

    let response = match http::Client::new().send(request).await {
        Ok(r) => r,
        Err(e) => {
            return Output::InternalError {
                reason: format!("upstream send: {e}"),
            };
        }
    };

    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body_bytes = match response.into_body().bytes_limited(usize::MAX).await {
        Ok(body_bytes) => body_bytes,
        Err(error) => {
            return Output::InternalError {
                reason: format!("read upstream body: {error}"),
            };
        }
    };
    let body = String::from_utf8_lossy(&body_bytes).into_owned();

    if (200..300).contains(&status) {
        Output::Ok {
            status,
            content_type,
            body,
        }
    } else {
        Output::UpstreamError {
            status,
            content_type,
            body,
        }
    }
}
