use std::{fmt::Display, ops::Deref};

#[derive(Eq, Hash, PartialEq, Clone, Debug, Ord, PartialOrd)]
pub struct HostId(String);

impl HostId {
    pub fn new(id: String) -> Self {
        Self(id)
    }
}
impl AsRef<str> for HostId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl Deref for HostId {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl From<String> for HostId {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl Display for HostId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
