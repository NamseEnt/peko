use crate::common::auth;
use crate::docs::*;
use forte_sdk::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Input {
    pub operation: String,
    pub account_id: String,
    pub project_id: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "t", rename_all_fields = "camelCase")]
pub enum Output {
    Authorized { github_id: i64 },
    NotLoggedIn,
    NotFound,
    InvalidRequest,
    InternalError,
}

const ALLOWED_OPERATIONS: [&str; 10] = [
    "resolve_zone",
    "provision_project",
    "ensure_websockets",
    "issue_origin_certificate",
    "finalize_domain",
    "revoke_project_credentials",
    "rotate_token",
    "clear_token",
    "destroy_broker",
    "teardown_project",
];

pub async fn handler(req: ForteRequest<'_, Input>) -> Output {
    let Some(user) = auth::bearer_user(req.headers).await else {
        return Output::NotLoggedIn;
    };
    authorize(
        &doc_db::turso(),
        &req.body.operation,
        &req.body.account_id,
        req.body.project_id.as_deref(),
        user.github_id,
    )
    .await
}

async fn authorize(
    db: &doc_db::Database,
    operation: &str,
    account_id: &str,
    project_id: Option<&str>,
    github_id: i64,
) -> Output {
    if account_id.is_empty() || !ALLOWED_OPERATIONS.contains(&operation) {
        return Output::InvalidRequest;
    }

    if let Some(project_id) = project_id {
        let project = match (ProjectDocGet { project_id }).send_with(db).await {
            Ok(Some(project)) => project,
            Ok(None) => return Output::NotFound,
            Err(error) => {
                tracing::error!(%error, "cloudflare_broker_authorize project lookup failed");
                return Output::InternalError;
            }
        };
        if project.owner_github_id != github_id {
            return Output::NotFound;
        }
    }

    Output::Authorized { github_id }
}

#[cfg(test)]
mod tests {
    use super::{Output, authorize};
    use crate::docs::{DbRequest, ProjectDoc, ProjectDocPut};

    const ACCOUNT_ID: &str = "account-id";

    #[test]
    fn refuses_an_operation_outside_the_fixed_set() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            let output = authorize(&db, "delete_zone", ACCOUNT_ID, None, 1).await;
            assert!(matches!(output, Output::InvalidRequest));
        });
    }

    #[test]
    fn refuses_an_empty_account_id() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            let output = authorize(&db, "resolve_zone", "", None, 1).await;
            assert!(matches!(output, Output::InvalidRequest));
        });
    }

    #[test]
    fn account_level_operations_need_no_project() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            let output = authorize(&db, "rotate_token", ACCOUNT_ID, None, 1).await;
            assert!(matches!(output, Output::Authorized { github_id: 1 }));
        });
    }

    #[test]
    fn authorizes_the_projects_owner() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            ProjectDocPut(ProjectDoc {
                project_id: "abcd1234".to_string(),
                owner_github_id: 7,
                name: "demo".to_string(),
                created_at: forte_sdk::now(),
            })
            .send_with(&db)
            .await
            .unwrap();
            let output = authorize(&db, "provision_project", ACCOUNT_ID, Some("abcd1234"), 7).await;
            assert!(matches!(output, Output::Authorized { github_id: 7 }));
        });
    }

    #[test]
    fn refuses_a_caller_who_does_not_own_the_project() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            ProjectDocPut(ProjectDoc {
                project_id: "abcd1234".to_string(),
                owner_github_id: 7,
                name: "demo".to_string(),
                created_at: forte_sdk::now(),
            })
            .send_with(&db)
            .await
            .unwrap();
            let output = authorize(&db, "provision_project", ACCOUNT_ID, Some("abcd1234"), 8).await;
            assert!(matches!(output, Output::NotFound));
        });
    }

    #[test]
    fn refuses_an_unknown_project() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            let output = authorize(&db, "provision_project", ACCOUNT_ID, Some("missing1"), 7).await;
            assert!(matches!(output, Output::NotFound));
        });
    }

    #[test]
    fn teardown_project_is_gated_on_project_ownership() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            ProjectDocPut(ProjectDoc {
                project_id: "abcd1234".to_string(),
                owner_github_id: 7,
                name: "demo".to_string(),
                created_at: forte_sdk::now(),
            })
            .send_with(&db)
            .await
            .unwrap();
            assert!(matches!(
                authorize(&db, "teardown_project", ACCOUNT_ID, Some("abcd1234"), 7).await,
                Output::Authorized { github_id: 7 }
            ));
            assert!(matches!(
                authorize(&db, "teardown_project", ACCOUNT_ID, Some("abcd1234"), 9).await,
                Output::NotFound
            ));
        });
    }
}
