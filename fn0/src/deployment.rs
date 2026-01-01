use std::collections::HashMap;

type CodeId = String;
type DeploymentId = String;

pub struct Deployment {
    pub id: DeploymentId,
    pub codes: HashMap<CodeId, CodeManifest>,
}

pub struct CodeManifest {
    pub kind: CodeKind,
    /// Codes can communicate with each other using this ID like internal://<code_id>
    pub code_id: CodeId,
}

#[derive(Clone, Copy)]
pub enum CodeKind {
    Wasm,
    Js,
}

#[derive(Default)]
pub struct DeploymentMap {
    code_id_deployment_id_map: HashMap<CodeId, DeploymentId>,
    code_manifest_map: HashMap<CodeId, CodeManifest>,
}

impl DeploymentMap {
    pub fn new() -> Self {
        Self {
            code_id_deployment_id_map: Default::default(),
            code_manifest_map: Default::default(),
        }
    }

    pub fn register_code(&mut self, code_id: &str, kind: CodeKind) {
        self.code_id_deployment_id_map
            .insert(code_id.to_string(), "default".to_string());
        self.code_manifest_map.insert(
            code_id.to_string(),
            CodeManifest {
                kind,
                code_id: code_id.to_string(),
            },
        );
    }

    pub fn is_code_in_same_deployment(
        &self,
        code_id_a: &CodeId,
        code_id_b: &CodeId,
    ) -> Option<bool> {
        Some(
            self.code_id_deployment_id_map.get(code_id_a)?
                == self.code_id_deployment_id_map.get(code_id_b)?,
        )
    }

    pub fn code_kind(&self, code_id: &str) -> Option<CodeKind> {
        self.code_manifest_map
            .get(code_id)
            .map(|manifest| manifest.kind)
    }
}
