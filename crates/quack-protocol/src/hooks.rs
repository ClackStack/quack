//! Declarative hook setup. Descriptors and service presets come from the bundle.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{KeySource, SecretText};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookDescriptor {
    pub version: u32,
    /// Connection value containing the web base used to scope access checks.
    pub scope_field: String,
    pub fields: Vec<HookField>,
    pub presets: Vec<HookPreset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookField {
    pub id: String,
    pub label: String,
    /// text, url, integer, boolean, choice, or secret.
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub advanced: bool,
    /// Changing this value requires an explicit credential change.
    #[serde(default)]
    pub credential_scope: bool,
    #[serde(default)]
    pub required_when: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookPreset {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    #[serde(default)]
    pub choices: BTreeMap<String, Vec<String>>,
    /// Field templates expanded from the connection values, e.g. {web_url}/api/v4.
    #[serde(default)]
    pub derived: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookConnection {
    pub id: String,
    pub preset: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub values: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookConnectionInfo {
    pub connection: HookConnection,
    pub credentials: BTreeMap<String, KeySource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookSetupInfo {
    pub id: String,
    pub revision: String,
    pub descriptor: HookDescriptor,
    pub connections: Vec<HookConnectionInfo>,
    pub enabled: bool,
    pub source: String,
    pub writable: bool,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HookCredentialChange {
    #[default]
    Keep,
    Remove,
    Replace {
        value: SecretText,
    },
    /// env, file, keyring, or cmd (a JSON argument array).
    External {
        source: String,
        location: SecretText,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookConnectionDraft {
    pub connection: HookConnection,
    #[serde(default)]
    pub credentials: BTreeMap<String, HookCredentialChange>,
}
