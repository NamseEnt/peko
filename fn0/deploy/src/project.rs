use crate::name::MAX_PROJECT_NAME_LEN;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct NewProjectInput<'a> {
    name: &'a str,
}

#[derive(Deserialize)]
#[serde(tag = "t", rename_all_fields = "camelCase")]
enum NewProject {
    Ok { project_id: String },
    NotLoggedIn,
    InvalidName,
    InternalError,
}

pub async fn ensure_project_id(
    client: &reqwest::Client,
    control_url: &str,
    token: &str,
    project_name: &str,
    project_id: &mut Option<String>,
) -> Result<String> {
    if let Some(id) = project_id.as_ref() {
        return Ok(id.clone());
    }
    let url = format!(
        "{}/__forte_action/new_project",
        control_url.trim_end_matches('/')
    );
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&NewProjectInput { name: project_name })
        .send()
        .await?
        .error_for_status()
        .map_err(|e| anyhow!("new_project failed: {e}"))?;
    let raw: NewProject = resp.json().await?;
    let id = match raw {
        NewProject::Ok { project_id } => project_id,
        NewProject::NotLoggedIn => {
            return Err(anyhow!("control rejected token; run `fn0 login` again."));
        }
        NewProject::InvalidName => {
            return Err(anyhow!(
                "control rejected project name '{project_name}': must be 1-{MAX_PROJECT_NAME_LEN} chars of letters, digits, '.', '_', '-'"
            ));
        }
        NewProject::InternalError => {
            return Err(anyhow!("new_project: server error; check fn0-control logs"));
        }
    };
    *project_id = Some(id.clone());
    Ok(id)
}

#[derive(Serialize)]
struct RenameProjectInput<'a> {
    project_id: &'a str,
    new_name: &'a str,
}

#[derive(Deserialize)]
#[serde(tag = "t", rename_all_fields = "camelCase")]
enum RenameProject {
    Ok,
    NotLoggedIn,
    NotFound,
    InvalidName,
    InternalError,
}

#[derive(Serialize)]
struct DeleteProjectInput<'a> {
    project_id: &'a str,
}

#[derive(Deserialize)]
#[serde(tag = "t", rename_all_fields = "camelCase")]
enum DeleteProject {
    Ok,
    NotLoggedIn,
    NotFound,
    InternalError,
}

async fn request_delete_project(project_id: &str) -> Result<DeleteProject> {
    let creds = crate::credentials::require()?;
    let client = reqwest::Client::new();
    let url = format!(
        "{}/__forte_action/delete_project",
        creds.control_url.trim_end_matches('/')
    );
    let resp = client
        .post(&url)
        .bearer_auth(&creds.token)
        .json(&DeleteProjectInput { project_id })
        .send()
        .await?
        .error_for_status()?;
    let raw: DeleteProject = resp.json().await?;
    Ok(raw)
}

pub async fn delete_project(project_id: &str) -> Result<()> {
    match request_delete_project(project_id).await? {
        DeleteProject::Ok => Ok(()),
        DeleteProject::NotLoggedIn => {
            Err(anyhow!("control rejected token; run `fn0 login` again."))
        }
        DeleteProject::NotFound => Err(anyhow!(
            "project '{project_id}' not found or not owned by you."
        )),
        DeleteProject::InternalError => Err(anyhow!(
            "delete_project: server error; check fn0-control logs"
        )),
    }
}

pub async fn delete_project_if_present(project_id: &str) -> Result<()> {
    match request_delete_project(project_id).await? {
        DeleteProject::Ok | DeleteProject::NotFound => Ok(()),
        DeleteProject::NotLoggedIn => {
            Err(anyhow!("control rejected token; run `fn0 login` again."))
        }
        DeleteProject::InternalError => Err(anyhow!(
            "delete_project: server error; check fn0-control logs"
        )),
    }
}

pub async fn rename_project(project_id: &str, new_name: &str) -> Result<()> {
    let creds = crate::credentials::require()?;
    let client = reqwest::Client::new();
    let url = format!(
        "{}/__forte_action/rename_project",
        creds.control_url.trim_end_matches('/')
    );
    let resp = client
        .post(&url)
        .bearer_auth(&creds.token)
        .json(&RenameProjectInput {
            project_id,
            new_name,
        })
        .send()
        .await?
        .error_for_status()?;
    let raw: RenameProject = resp.json().await?;
    match raw {
        RenameProject::Ok => Ok(()),
        RenameProject::NotLoggedIn => {
            Err(anyhow!("control rejected token; run `fn0 login` again."))
        }
        RenameProject::NotFound => Err(anyhow!(
            "project '{project_id}' not found or not owned by you."
        )),
        RenameProject::InvalidName => Err(anyhow!(
            "control rejected name '{new_name}': must be 1-{MAX_PROJECT_NAME_LEN} chars of letters, digits, '.', '_', '-'"
        )),
        RenameProject::InternalError => Err(anyhow!(
            "rename_project: server error; check fn0-control logs"
        )),
    }
}
