use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_STRUCTURED_HANDOFF_BYTES: usize = 16 * 1024;
pub const MAX_FINDINGS: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StructuredHandoff {
    pub version: u32,
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub verification: Vec<VerificationEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Finding {
    pub id: String,
    pub status: FindingStatus,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FindingStatus {
    Open,
    Resolved,
    Accepted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerificationEvidence {
    pub description: String,
    pub result: String,
}

impl StructuredHandoff {
    pub fn parse(input: &[u8]) -> Result<Self, String> {
        if input.len() > MAX_STRUCTURED_HANDOFF_BYTES {
            return Err(format!(
                "Structured handoff exceeds {MAX_STRUCTURED_HANDOFF_BYTES} bytes"
            ));
        }
        let handoff: Self = serde_json::from_slice(input)
            .map_err(|error| format!("Invalid structured handoff: {error}"))?;
        handoff.validate()?;
        Ok(handoff)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!(
                "Unsupported structured handoff version: {}",
                self.version
            ));
        }
        if self.summary.trim().is_empty() {
            return Err("Structured handoff summary must not be blank".into());
        }
        if self.findings.len() > MAX_FINDINGS {
            return Err(format!(
                "Structured handoff has more than {MAX_FINDINGS} findings"
            ));
        }
        let mut ids = HashSet::with_capacity(self.findings.len());
        for finding in &self.findings {
            if finding.id.trim().is_empty() || !ids.insert(&finding.id) {
                return Err("Finding IDs must be nonblank and unique".into());
            }
            if finding.description.trim().is_empty() {
                return Err(format!(
                    "Finding {} description must not be blank",
                    finding.id
                ));
            }
        }
        for evidence in &self.verification {
            if evidence.description.trim().is_empty() || evidence.result.trim().is_empty() {
                return Err(
                    "Verification evidence description and result must not be blank".into(),
                );
            }
        }
        if self
            .next_action
            .as_ref()
            .is_some_and(|action| action.trim().is_empty())
        {
            return Err("Next action must not be blank when provided".into());
        }
        let bytes = serde_json::to_vec(self)
            .map_err(|error| error.to_string())?
            .len();
        if bytes > MAX_STRUCTURED_HANDOFF_BYTES {
            return Err(format!(
                "Structured handoff exceeds {MAX_STRUCTURED_HANDOFF_BYTES} bytes"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> StructuredHandoff {
        StructuredHandoff {
            version: 1,
            summary: "Build complete".into(),
            findings: vec![Finding {
                id: "F-1".into(),
                status: FindingStatus::Open,
                description: "Add coverage".into(),
            }],
            verification: vec![VerificationEvidence {
                description: "cargo test".into(),
                result: "passed".into(),
            }],
            next_action: None,
        }
    }

    #[test]
    fn accepts_valid_handoff_and_rejects_bad_version_and_duplicate_ids() {
        assert!(valid().validate().is_ok());
        let mut handoff = valid();
        handoff.version = 2;
        assert!(handoff.validate().unwrap_err().contains("version"));
        let mut handoff = valid();
        handoff.findings.push(handoff.findings[0].clone());
        assert!(handoff.validate().unwrap_err().contains("unique"));
    }

    #[test]
    fn parse_rejects_unknown_fields_malformed_and_oversized_input() {
        let unknown = br#"{"version":1,"summary":"ok","surprise":"discarded"}"#;
        assert!(
            StructuredHandoff::parse(unknown)
                .unwrap_err()
                .contains("unknown field")
        );
        assert!(
            StructuredHandoff::parse(b"{")
                .unwrap_err()
                .contains("Invalid")
        );
        let oversized = format!(
            "{{\"version\":1,\"summary\":\"{}\"}}",
            "x".repeat(MAX_STRUCTURED_HANDOFF_BYTES)
        );
        assert!(
            StructuredHandoff::parse(oversized.as_bytes())
                .unwrap_err()
                .contains("exceeds")
        );
    }

    #[test]
    fn rejects_blank_required_content_and_payload_over_limit() {
        let mut handoff = valid();
        handoff.summary = "  ".into();
        assert!(handoff.validate().unwrap_err().contains("summary"));
        let mut handoff = valid();
        handoff.summary = "x".repeat(MAX_STRUCTURED_HANDOFF_BYTES);
        assert!(handoff.validate().unwrap_err().contains("exceeds"));
    }
}
