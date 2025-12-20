use std::collections::BTreeMap;

#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct HqArgs {
    pub providers: Vec<HostProviderVariant>,
}

#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "type")]
pub enum HostProviderVariant {
    OciContainerInstance(OciContainerInstanceHostProviderArgs),
}

#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct OciContainerInstanceHostProviderArgs {
    pub private_key_base64: String,
    pub user_id: String,
    pub fingerprint: String,
    pub tenancy_id: String,
    pub region: String,
    pub compartment_id: String,
    pub availability_domain: String,
    pub shape: String,
    pub ocpus: f32,
    pub memory_in_gbs: f32,
    pub subnet_id: String,
    pub image: String,
    pub envs: BTreeMap<String, String>,
}
