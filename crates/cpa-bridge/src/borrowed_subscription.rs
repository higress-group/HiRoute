//! Shared safe evidence surface; native credential payloads stay inside their adapters.
use crate::{BorrowedClaudeEvidence, BorrowedCodexEvidence, CpaAccountKind};
use hiroute_domain::CanonicalDigest;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BorrowedSubscriptionEvidence {
    Codex(BorrowedCodexEvidence),
    Claude(BorrowedClaudeEvidence),
}
impl From<BorrowedCodexEvidence> for BorrowedSubscriptionEvidence {
    fn from(value: BorrowedCodexEvidence) -> Self {
        Self::Codex(value)
    }
}
impl From<BorrowedClaudeEvidence> for BorrowedSubscriptionEvidence {
    fn from(value: BorrowedClaudeEvidence) -> Self {
        Self::Claude(value)
    }
}
impl BorrowedSubscriptionEvidence {
    pub fn kind(&self) -> CpaAccountKind {
        match self {
            Self::Codex(_) => CpaAccountKind::Codex,
            Self::Claude(_) => CpaAccountKind::Claude,
        }
    }
    pub fn account_ref(&self) -> String {
        match self {
            Self::Codex(v) => v.account_ref(),
            Self::Claude(v) => v.account_ref(),
        }
    }
    pub fn evidence_digest(&self) -> &CanonicalDigest {
        match self {
            Self::Codex(v) => v.evidence_digest(),
            Self::Claude(v) => v.evidence_digest(),
        }
    }
    pub fn binding_evidence_digest(&self) -> &CanonicalDigest {
        match self {
            Self::Codex(v) => v.binding_evidence_digest(),
            Self::Claude(v) => v.binding_evidence_digest(),
        }
    }
    pub(crate) fn codex(&self) -> Option<&BorrowedCodexEvidence> {
        match self {
            Self::Codex(v) => Some(v),
            _ => None,
        }
    }
    pub(crate) fn claude(&self) -> Option<&BorrowedClaudeEvidence> {
        match self {
            Self::Claude(v) => Some(v),
            _ => None,
        }
    }
}
