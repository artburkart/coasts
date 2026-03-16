use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use ts_rs::TS;

/// Container runtime type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeType {
    Dind,
    Sysbox,
    Podman,
}

impl RuntimeType {
    pub fn from_str_value(s: &str) -> Option<Self> {
        match s {
            "dind" => Some(Self::Dind),
            "sysbox" => Some(Self::Sysbox),
            "podman" => Some(Self::Podman),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dind => "dind",
            Self::Sysbox => "sysbox",
            Self::Podman => "podman",
        }
    }
}

impl std::fmt::Display for RuntimeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Configuration for customizing the coast container itself.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SetupConfig {
    pub packages: Vec<String>,
    pub run: Vec<String>,
    #[serde(default)]
    pub files: Vec<SetupFileConfig>,
}

/// A host directory bind-mounted into the coast container at runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostMountConfig {
    pub name: String,
    pub source: PathBuf,
    pub target: String,
    #[serde(default = "default_true")]
    pub read_only: bool,
}

/// A file to materialize inside the coast image during `[coast.setup]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupFileConfig {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub mode: Option<String>,
}

impl SetupConfig {
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty() && self.run.is_empty() && self.files.is_empty()
    }
}

/// Host file/env injection configuration (non-secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInjectConfig {
    pub env: Vec<String>,
    pub files: Vec<String>,
}

const fn default_true() -> bool {
    true
}
