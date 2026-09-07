//! Reclaims the R2 prefixes of superseded code versions for one project.
//!
//! The `{code_version}/` prefixes grow with every deploy and nothing else
//! removes them: the frontend-asset bucket holds the deployed build.
//!
//! Enqueued once a deploy reaches `static_cache_state == active`, and it
//! re-reads the manifest rather than trusting the payload, so a redelivered
//! message never prunes against a stale active version.

use crate::common::aws_sign::R2ListedObject;
use crate::common::byoc::ProjectStorage;
use crate::common::r2_store::ProjectR2Store;
use crate::docs::*;
use fn0_shared_schema::STATIC_CACHE_STATE_ACTIVE;
use forte_sdk::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

#[derive(Serialize, Deserialize)]
pub struct Input {
    pub project_id: String,
}

pub async fn handle(input: Input) -> anyhow::Result<()> {
    let db = doc_db::turso();
    let Some(manifest) = (WorkerManifestDocGet {}).send_with(&db).await? else {
        return Ok(());
    };
    let Some(entry) = manifest.project_manifests.get(&input.project_id) else {
        return Ok(());
    };
    // Mid-activation the fleet is split across two code versions, and which
    // one a given worker serves is not observable from here.
    if entry.static_cache_state != STATIC_CACHE_STATE_ACTIVE {
        return Ok(());
    }
    let active_code_version = entry.code_version;
    prune_singleton_configs(&db, &input.project_id, active_code_version).await?;
    let Some(manifest) = (WorkerManifestDocGet {}).send_with(&db).await? else {
        return Ok(());
    };
    if !manifest
        .project_manifests
        .get(&input.project_id)
        .is_some_and(|entry| {
            entry.code_version == active_code_version
                && entry.static_cache_state == STATIC_CACHE_STATE_ACTIVE
        })
    {
        return Ok(());
    }
    let now = forte_sdk::now();
    let storage = ProjectStorage::resolve(&db, &input.project_id).await?;

    let mut deleted = 0usize;
    let store = ProjectR2Store::frontend_assets(&storage);
    for key in prunable_keys(&store.list_all("", now).await?, active_code_version) {
        store.delete(&key, now).await?;
        deleted += 1;
    }

    tracing::info!(
        project_id = %input.project_id,
        active_code_version,
        deleted,
        "deploy_artifact_prune complete"
    );
    Ok(())
}

/// Keeps every code version at or above the active one, because a deploy
/// uploads its assets before it activates, plus the newest versions below it,
/// because a page already loaded from an earlier deploy is still fetching that
/// deploy's assets. Keys that do not carry a parseable code version are left
/// alone.
///
/// Two versions below active rather than one: a deploy that fails after
/// uploading assets leaves a prefix that never activates, and with a single
/// retained version that dead prefix would evict the version clients are
/// actually still loading from.
const RETAINED_VERSIONS_BELOW_ACTIVE: usize = 2;
const SINGLETON_CONFIG_PAGE_SIZE: usize = 64;

#[derive(Debug, Default, PartialEq, Eq)]
struct ConfigPruneCounts {
    scanned: usize,
    retained: usize,
    deleted: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum ConfigDeleteResult {
    Deleted,
    Retained,
    DeploymentChanged,
}

async fn singleton_config_versions(
    db: &doc_db::Database,
    project_id: &str,
) -> anyhow::Result<Vec<u64>> {
    let mut versions = Vec::new();
    let mut after_code_version = None;
    loop {
        let configs = (WebSocketSingletonConfigDocQuery {
            project_id,
            code_version: after_code_version,
            limit: Some(SINGLETON_CONFIG_PAGE_SIZE),
        })
        .send_with(db)
        .await?;
        let page_len = configs.len();
        after_code_version = configs.last().map(|config| config.code_version);
        versions.extend(configs.into_iter().map(|config| config.code_version));
        if page_len < SINGLETON_CONFIG_PAGE_SIZE {
            return Ok(versions);
        }
    }
}

async fn prune_singleton_configs(
    db: &doc_db::Database,
    project_id: &str,
    active_code_version: u64,
) -> anyhow::Result<ConfigPruneCounts> {
    let versions = singleton_config_versions(db, project_id).await?;
    let retained = retained_versions(versions.iter().copied(), active_code_version);
    let mut counts = ConfigPruneCounts {
        scanned: versions.len(),
        retained: versions.len(),
        deleted: 0,
    };
    let mut deployment_changed = false;
    for code_version in versions {
        if code_version >= active_code_version || retained.contains(&code_version) {
            continue;
        }
        match delete_singleton_config(db, project_id, active_code_version, code_version).await? {
            ConfigDeleteResult::Deleted => {
                counts.deleted += 1;
                counts.retained -= 1;
            }
            ConfigDeleteResult::Retained => {}
            ConfigDeleteResult::DeploymentChanged => {
                deployment_changed = true;
                break;
            }
        }
    }
    tracing::info!(
        project_id,
        active_code_version,
        scanned = counts.scanned,
        retained = counts.retained,
        deleted = counts.deleted,
        deployment_changed,
        "websocket singleton config prune complete"
    );
    Ok(counts)
}

async fn delete_singleton_config(
    db: &doc_db::Database,
    project_id: &str,
    active_code_version: u64,
    code_version: u64,
) -> anyhow::Result<ConfigDeleteResult> {
    let result = db
        .trx(|trx| async move {
            let (manifest, config) = trx
                .get((
                    WorkerManifestDocGet {},
                    WebSocketSingletonConfigDocGet {
                        project_id,
                        code_version,
                    },
                ))
                .await?;
            let entry = manifest
                .as_ref()
                .and_then(|manifest| manifest.project_manifests.get(project_id));
            if !entry.is_some_and(|entry| {
                entry.code_version == active_code_version
                    && entry.static_cache_state == STATIC_CACHE_STATE_ACTIVE
            }) {
                return trx.cancel(ConfigDeleteResult::DeploymentChanged);
            }
            let Some(config) = config else {
                return trx.commit(ConfigDeleteResult::Retained);
            };
            if config.project_id != project_id
                || config.code_version != code_version
                || code_version >= active_code_version
            {
                return trx.cancel(ConfigDeleteResult::Retained);
            }
            config.delete();
            trx.commit(ConfigDeleteResult::Deleted)
        })
        .await;
    match result {
        doc_db::TrxResult::Committed(outcome) | doc_db::TrxResult::Cancelled(outcome) => {
            Ok(outcome)
        }
        doc_db::TrxResult::Conflict(error) => {
            anyhow::bail!("websocket singleton config prune conflict: {error:?}")
        }
        doc_db::TrxResult::Err(error) => Err(error),
    }
}

fn retained_versions(
    versions: impl Iterator<Item = u64>,
    active_code_version: u64,
) -> HashSet<u64> {
    versions
        .filter(|code_version| *code_version < active_code_version)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .rev()
        .take(RETAINED_VERSIONS_BELOW_ACTIVE)
        .collect()
}

fn prunable_keys(objects: &[R2ListedObject], active_code_version: u64) -> Vec<String> {
    let retained = retained_versions(
        objects
            .iter()
            .filter_map(|object| code_version_of(&object.key)),
        active_code_version,
    );

    objects
        .iter()
        .filter(|object| match code_version_of(&object.key) {
            Some(code_version) => {
                code_version < active_code_version && !retained.contains(&code_version)
            }
            None => false,
        })
        .map(|object| object.key.clone())
        .collect()
}

fn code_version_of(key: &str) -> Option<u64> {
    key.split('/').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        ConfigDeleteResult, ConfigPruneCounts, SINGLETON_CONFIG_PAGE_SIZE, delete_singleton_config,
        prunable_keys, prune_singleton_configs, singleton_config_versions,
    };
    use crate::common::aws_sign::R2ListedObject;
    use crate::docs::*;
    use fn0_shared_schema::{STATIC_CACHE_STATE_ACTIVATING, STATIC_CACHE_STATE_ACTIVE};
    use std::collections::HashMap;

    async fn put_manifest(db: &doc_db::Database, code_version: u64, state: &str) {
        WorkerManifestDocPut(WorkerManifestDoc {
            manifest_version: code_version,
            project_manifests: HashMap::from([(
                "project".to_string(),
                WorkerProjectManifest {
                    code_version,
                    domain: String::new(),
                    static_cache_state: state.to_string(),
                    pending_code_version: None,
                    storage: None,
                },
            )]),
        })
        .send_with(db)
        .await
        .unwrap();
    }

    async fn put_configs(db: &doc_db::Database, project_id: &str, versions: &[u64]) {
        for &code_version in versions {
            WebSocketSingletonConfigDocPut(WebSocketSingletonConfigDoc {
                project_id: project_id.to_string(),
                code_version,
                declarations: Vec::new(),
            })
            .send_with(db)
            .await
            .unwrap();
        }
    }

    async fn config_versions(db: &doc_db::Database, project_id: &str) -> Vec<u64> {
        let mut versions = singleton_config_versions(db, project_id).await.unwrap();
        versions.sort_unstable();
        versions
    }

    #[test]
    fn prunes_paginated_empty_configs_and_preserves_other_projects() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            let active_code_version = (SINGLETON_CONFIG_PAGE_SIZE * 2) as u64;
            let versions: Vec<u64> = (0..=active_code_version + 2).collect();
            put_manifest(&db, active_code_version, STATIC_CACHE_STATE_ACTIVE).await;
            put_configs(&db, "project", &versions).await;
            put_configs(&db, "other", &versions).await;

            let counts = prune_singleton_configs(&db, "project", active_code_version)
                .await
                .unwrap();

            assert_eq!(counts.scanned, versions.len());
            assert_eq!(counts.retained, 5);
            assert_eq!(counts.deleted, versions.len() - 5);
            assert_eq!(
                config_versions(&db, "project").await,
                (active_code_version - 2..=active_code_version + 2).collect::<Vec<_>>()
            );
            assert_eq!(config_versions(&db, "other").await, versions);

            assert_eq!(
                prune_singleton_configs(&db, "project", active_code_version)
                    .await
                    .unwrap(),
                ConfigPruneCounts {
                    scanned: 5,
                    retained: 5,
                    deleted: 0,
                }
            );
        });
    }

    #[test]
    fn repeated_deployments_keep_only_three_empty_configs() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            for code_version in 1..=12 {
                put_configs(&db, "project", &[code_version]).await;
                put_manifest(&db, code_version, STATIC_CACHE_STATE_ACTIVE).await;
                prune_singleton_configs(&db, "project", code_version)
                    .await
                    .unwrap();
                assert!(config_versions(&db, "project").await.len() <= 3);
            }
            assert_eq!(config_versions(&db, "project").await, vec![10, 11, 12]);
        });
    }

    #[test]
    fn failed_deploy_is_pruned_after_later_activations_pass_retained_history() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            put_configs(&db, "project", &[10, 20]).await;
            put_manifest(&db, 10, STATIC_CACHE_STATE_ACTIVE).await;
            prune_singleton_configs(&db, "project", 10).await.unwrap();
            assert_eq!(config_versions(&db, "project").await, vec![10, 20]);

            for code_version in [30, 40, 50] {
                put_configs(&db, "project", &[code_version]).await;
                put_manifest(&db, code_version, STATIC_CACHE_STATE_ACTIVE).await;
                prune_singleton_configs(&db, "project", code_version)
                    .await
                    .unwrap();
            }
            assert_eq!(config_versions(&db, "project").await, vec![30, 40, 50]);
        });
    }

    #[test]
    fn deletion_rechecks_activation_after_the_scan() {
        futures::executor::block_on(async {
            for (current_code_version, state) in [
                (40, STATIC_CACHE_STATE_ACTIVATING),
                (50, STATIC_CACHE_STATE_ACTIVE),
                (10, STATIC_CACHE_STATE_ACTIVE),
            ] {
                let db = doc_db::memory();
                put_configs(&db, "project", &[10, 20, 30, 40]).await;
                put_manifest(&db, 40, STATIC_CACHE_STATE_ACTIVE).await;
                assert_eq!(config_versions(&db, "project").await, vec![10, 20, 30, 40]);

                put_configs(&db, "project", &[50]).await;
                put_manifest(&db, current_code_version, state).await;
                assert_eq!(
                    delete_singleton_config(&db, "project", 40, 10)
                        .await
                        .unwrap(),
                    ConfigDeleteResult::DeploymentChanged
                );
                assert_eq!(
                    prune_singleton_configs(&db, "project", 40)
                        .await
                        .unwrap()
                        .deleted,
                    0
                );
                assert_eq!(
                    config_versions(&db, "project").await,
                    vec![10, 20, 30, 40, 50]
                );
            }
        });
    }

    #[test]
    fn deletion_preserves_a_newer_deploy_written_after_the_scan() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            put_configs(&db, "project", &[10, 20, 30, 40]).await;
            put_manifest(&db, 40, STATIC_CACHE_STATE_ACTIVE).await;
            assert_eq!(config_versions(&db, "project").await, vec![10, 20, 30, 40]);
            put_configs(&db, "project", &[50]).await;

            assert_eq!(
                delete_singleton_config(&db, "project", 40, 10)
                    .await
                    .unwrap(),
                ConfigDeleteResult::Deleted
            );
            for code_version in [10, 40, 50] {
                assert_eq!(
                    delete_singleton_config(&db, "project", 40, code_version)
                        .await
                        .unwrap(),
                    ConfigDeleteResult::Retained
                );
            }
            assert_eq!(config_versions(&db, "project").await, vec![20, 30, 40, 50]);
        });
    }

    #[test]
    fn missing_manifest_or_project_stops_deletion() {
        futures::executor::block_on(async {
            let db = doc_db::memory();
            put_configs(&db, "project", &[10]).await;
            assert_eq!(
                delete_singleton_config(&db, "project", 40, 10)
                    .await
                    .unwrap(),
                ConfigDeleteResult::DeploymentChanged
            );
            WorkerManifestDocPut(WorkerManifestDoc {
                manifest_version: 40,
                project_manifests: HashMap::new(),
            })
            .send_with(&db)
            .await
            .unwrap();
            assert_eq!(
                delete_singleton_config(&db, "project", 40, 10)
                    .await
                    .unwrap(),
                ConfigDeleteResult::DeploymentChanged
            );
            assert_eq!(config_versions(&db, "project").await, vec![10]);
        });
    }

    fn object(key: &str) -> R2ListedObject {
        R2ListedObject {
            key: key.to_string(),
            size: 0,
            last_modified: forte_sdk::chrono::DateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn keeps_active_and_two_below_and_prunes_older() {
        let objects = vec![
            object("100/client.js"),
            object("200/client.js"),
            object("300/client.js"),
            object("400/client.js"),
            object("400/__forte/pages/AB.html"),
        ];
        assert_eq!(
            prunable_keys(&objects, 400),
            vec!["100/client.js".to_string()]
        );
    }

    // The failed deploy leaves a prefix that never activates; it must not
    // evict the version clients are still loading from.
    #[test]
    fn a_failed_deploy_below_active_does_not_evict_the_live_previous_version() {
        let objects = vec![
            object("100/client.js"),
            object("200/client.js"),
            object("300/partial-upload.js"),
            object("400/client.js"),
        ];
        assert_eq!(
            prunable_keys(&objects, 400),
            vec!["100/client.js".to_string()]
        );
    }

    #[test]
    fn keeps_versions_newer_than_active() {
        let objects = vec![
            object("100/client.js"),
            object("200/client.js"),
            object("300/client.js"),
        ];
        assert_eq!(
            prunable_keys(&objects, 200),
            Vec::<String>::new(),
            "300 is a deploy that uploaded before activating"
        );
    }

    #[test]
    fn leaves_keys_without_a_code_version() {
        let objects = vec![object("not-a-version/client.js"), object("x")];
        assert_eq!(prunable_keys(&objects, 300), Vec::<String>::new());
    }
}
