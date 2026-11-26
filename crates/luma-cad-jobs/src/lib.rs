//! Provider-independent preparation and SQLite job outbox for CAD.
mod host;
mod plan;
mod queue;

pub use host::JobsHost;
pub use plan::{FutureOutput, Intent, Plan, Policy, SourcePin, TaskContract, TaskSpec};
pub use queue::{
    admit, attempt_receipt, cancel, capture, claim, complete, current, fail, install, install_in,
    intent, publish, receipt, retry, set_head, slot_generation, Artifact, Attempt, CancelOutcome,
    Claimed, Current, Delivery, JobId, JobReceipt, Publication, Scope, State,
};

#[derive(Debug)]
pub enum Error {
    Invalid(&'static str),
    Missing,
    Conflict,
    Corrupt,
    Database(rusqlite::Error),
    Json(serde_json::Error),
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "Invalid job input: {reason}"),
            Self::Missing => f.write_str("Job is absent"),
            Self::Conflict => f.write_str("Job or attempt has changed"),
            Self::Corrupt => f.write_str("Invalid persisted job state"),
            Self::Database(error) => write!(f, "Job storage failed: {error}"),
            Self::Json(error) => write!(f, "Job encoding failed: {error}"),
        }
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

fn stable_key(value: &str) -> bool {
    value.starts_with(|c: char| c.is_ascii_lowercase()) && scope_id(value)
}

fn scope_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= 128
        && matches!(bytes.next(), Some(b'a'..=b'z' | b'0'..=b'9'))
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.:/-".contains(&byte)
        })
}

fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}
