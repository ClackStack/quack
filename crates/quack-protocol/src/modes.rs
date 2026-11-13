//! Work mode and approval policy are independent session settings.

use core::fmt;
use core::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the agent may do. Planning never authorizes changes or test execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WorkMode {
    Plan,
    #[default]
    Edit,
}

impl WorkMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Edit => "edit",
        }
    }
}

/// Approval defaults, ordered so the existing system policy can cap them.
/// Read-only and auto-edit remain available for existing configurations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalPolicy {
    ReadOnly,
    #[default]
    Ask,
    AutoEdit,
    #[serde(alias = "full-auto")]
    Allow,
    /// Allow without the OS sandbox; work-mode restrictions and denies still apply.
    Yolo,
}

impl ApprovalPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Ask => "ask",
            Self::AutoEdit => "auto-edit",
            Self::Allow => "allow",
            Self::Yolo => "yolo",
        }
    }
}

/// The legacy compatibility value of `Op::SetPermissionMode` and `Event::PermissionMode`:
/// either a work mode (`plan`/`edit`) or an approval policy, spelled as the engine has
/// always serialized it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    Plan,
    Edit,
    ReadOnly,
    Ask,
    AutoEdit,
    #[serde(alias = "full-auto")]
    Allow,
    Yolo,
}

impl PermissionMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Edit => "edit",
            Self::ReadOnly => "read-only",
            Self::Ask => "ask",
            Self::AutoEdit => "auto-edit",
            Self::Allow => "allow",
            Self::Yolo => "yolo",
        }
    }

    /// The approval policy this mode selects, if it is one.
    pub fn policy(self) -> Option<ApprovalPolicy> {
        match self {
            Self::ReadOnly => Some(ApprovalPolicy::ReadOnly),
            Self::Ask => Some(ApprovalPolicy::Ask),
            Self::AutoEdit => Some(ApprovalPolicy::AutoEdit),
            Self::Allow => Some(ApprovalPolicy::Allow),
            Self::Yolo => Some(ApprovalPolicy::Yolo),
            Self::Plan | Self::Edit => None,
        }
    }
}

impl From<ApprovalPolicy> for PermissionMode {
    fn from(policy: ApprovalPolicy) -> Self {
        match policy {
            ApprovalPolicy::ReadOnly => Self::ReadOnly,
            ApprovalPolicy::Ask => Self::Ask,
            ApprovalPolicy::AutoEdit => Self::AutoEdit,
            ApprovalPolicy::Allow => Self::Allow,
            ApprovalPolicy::Yolo => Self::Yolo,
        }
    }
}

impl FromStr for PermissionMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "plan" => Ok(Self::Plan),
            "edit" => Ok(Self::Edit),
            "read-only" => Ok(Self::ReadOnly),
            "ask" => Ok(Self::Ask),
            "auto-edit" => Ok(Self::AutoEdit),
            "allow" | "full-auto" => Ok(Self::Allow),
            "yolo" => Ok(Self::Yolo),
            _ => Err(()),
        }
    }
}

impl fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

// Include the accepted legacy spelling in editor validation as well as serde.
impl JsonSchema for ApprovalPolicy {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ApprovalPolicy".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "enum": ["read-only", "ask", "auto-edit", "allow", "full-auto", "yolo"],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_auto_remains_an_accepted_alias_for_allow() {
        assert_eq!(
            serde_json::from_str::<ApprovalPolicy>("\"full-auto\"").expect("legacy policy"),
            ApprovalPolicy::Allow
        );
        assert_eq!(
            serde_json::to_string(&ApprovalPolicy::Allow).expect("policy"),
            "\"allow\""
        );
        let schema = schemars::schema_for!(ApprovalPolicy);
        assert!(
            schema.to_value()["enum"]
                .as_array()
                .expect("values")
                .contains(&serde_json::json!("full-auto"))
        );
    }
}

#[cfg(test)]
mod permission_mode_tests {
    use super::PermissionMode;

    #[test]
    fn serialization_matches_the_legacy_strings() {
        for (mode, wire) in [
            (PermissionMode::Plan, "plan"),
            (PermissionMode::Edit, "edit"),
            (PermissionMode::ReadOnly, "read-only"),
            (PermissionMode::Ask, "ask"),
            (PermissionMode::AutoEdit, "auto-edit"),
            (PermissionMode::Allow, "allow"),
            (PermissionMode::Yolo, "yolo"),
        ] {
            assert_eq!(
                serde_json::to_string(&mode).expect("mode"),
                format!("\"{wire}\"")
            );
            assert_eq!(mode.label(), wire);
            assert_eq!(mode.to_string(), wire);
            assert_eq!(wire.parse::<PermissionMode>().expect("mode"), mode);
        }
        assert_eq!(
            "full-auto".parse::<PermissionMode>().expect("mode"),
            PermissionMode::Allow
        );
    }
}
