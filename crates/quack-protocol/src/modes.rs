//! Work mode and approval policy are independent session settings.

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
