//! Typed errors for every IPC command. They serialise as
//! `{ "kind": "...", "message": "...", "details": [...], "fields": [...] }` so the UI can map `kind` to a
//! translated text.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FieldError {
    pub key: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("no game folder is configured")]
    NoRoot,
    #[error("{path}: {message}")]
    Io { path: String, message: String },
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("{} invalid value(s)", .0.len())]
    Validation(Vec<FieldError>),
    #[error("the game install is incomplete: missing {}", .0.join(", "))]
    NotInstalled(Vec<String>),
    #[error("the game is already running")]
    AlreadyRunning,
    #[error("the game is not running")]
    NotRunning,
    /// An update is putting the new executable (or translator) of this game in place: a few seconds.
    #[error("the game is being replaced by its new version")]
    GameUpdating,
    #[error("feature disabled: {0}")]
    FeatureDisabled(String),
    #[error("{0}")]
    ModeConflict(String),
    #[error("not available yet: {0}")]
    NotAvailable(String),
    /// A new save needs the whole appearance; the profile lacks these fields.
    #[error("the profile lacks {}", .0.join(", "))]
    ProfileIncomplete(Vec<String>),
    #[error("the profile is not valid: {0}")]
    ProfileInvalid(String),
    /// The launcher's own runtime source (embedded archive) is unusable: the program file is damaged or incomplete.
    #[error("Launcher files incomplete: download Kalamar again ({0})")]
    RuntimeMissing(String),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::NoRoot => "no_root",
            AppError::Io { .. } => "io",
            AppError::InvalidInput(_) => "invalid_input",
            AppError::Validation(_) => "validation",
            AppError::NotInstalled(_) => "not_installed",
            AppError::AlreadyRunning => "already_running",
            AppError::NotRunning => "not_running",
            AppError::GameUpdating => "game_updating",
            AppError::FeatureDisabled(_) => "feature_disabled",
            AppError::ModeConflict(_) => "mode_conflict",
            AppError::NotAvailable(_) => "not_available",
            AppError::ProfileIncomplete(_) => "profile_incomplete",
            AppError::ProfileInvalid(_) => "profile_invalid",
            AppError::RuntimeMissing(_) => "runtime_missing",
            AppError::Internal(_) => "internal",
        }
    }

    pub fn io(path: &std::path::Path, e: &std::io::Error) -> Self {
        AppError::Io { path: path.display().to_string(), message: e.to_string() }
    }
}

#[derive(Serialize)]
struct Wire {
    kind: &'static str,
    message: String,
    details: Vec<String>,
    fields: Vec<FieldError>,
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let details = match self {
            AppError::NotInstalled(v) | AppError::ProfileIncomplete(v) => v.clone(),
            AppError::ProfileInvalid(d) => vec![d.clone()],
            _ => Vec::new(),
        };
        let fields = match self {
            AppError::Validation(v) => v.clone(),
            _ => Vec::new(),
        };
        Wire { kind: self.kind(), message: self.to_string(), details, fields }.serialize(s)
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_with_kind_and_fields() {
        let e = AppError::Validation(vec![FieldError { key: "fps_limit".into(), message: "bad".into() }]);
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "validation");
        assert_eq!(v["fields"][0]["key"], "fps_limit");
    }

    #[test]
    fn not_installed_lists_missing() {
        let e = AppError::NotInstalled(vec!["exe".into(), "romfs".into()]);
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["details"][1], "romfs");
        assert!(v["message"].as_str().unwrap().contains("exe, romfs"));
    }
}
