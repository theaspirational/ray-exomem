use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(skip)]
    status: u16,
}

impl ApiError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
            actor: None,
            branch: None,
            suggestion: None,
            status: 400,
        }
    }
    pub fn with_path(mut self, p: impl Into<String>) -> Self {
        self.path = Some(p.into());
        self
    }
    pub fn with_actor(mut self, a: impl Into<String>) -> Self {
        self.actor = Some(a.into());
        self
    }
    pub fn with_branch(mut self, b: impl Into<String>) -> Self {
        self.branch = Some(b.into());
        self
    }
    pub fn with_suggestion(mut self, s: impl Into<String>) -> Self {
        self.suggestion = Some(s.into());
        self
    }
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// Convert to an HTTP (status, json-body) pair for the hand-rolled server.
    pub fn into_http_pair(self) -> (u16, String) {
        let body = serde_json::to_string(&self).unwrap_or_else(|_| {
            r#"{"code":"serialize_error","message":"failed to serialize error"}"#.to_string()
        });
        (self.status, body)
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = match self.status {
            401 => axum::http::StatusCode::UNAUTHORIZED,
            403 => axum::http::StatusCode::FORBIDDEN,
            404 => axum::http::StatusCode::NOT_FOUND,
            409 => axum::http::StatusCode::CONFLICT,
            410 => axum::http::StatusCode::GONE,
            501 => axum::http::StatusCode::NOT_IMPLEMENTED,
            400 => axum::http::StatusCode::BAD_REQUEST,
            _ => axum::http::StatusCode::BAD_REQUEST,
        };
        (status, axum::Json(&self)).into_response()
    }
}

/// Convert the internal WriteError into a structured ApiError.
impl From<crate::brain::WriteError> for ApiError {
    fn from(e: crate::brain::WriteError) -> Self {
        use crate::brain::WriteError::*;
        match e {
            NoSuchExom(p) => ApiError::new("no_such_exom", format!("no such exom {p}"))
                .with_path(p.clone())
                .with_suggestion(format!("ray-exomem init {p}")),
            SessionClosed => ApiError::new("session_closed", "session closed")
                .with_suggestion("retract session/closed_at to reopen"),
            BranchMissing(b) => {
                ApiError::new("branch_not_in_exom", format!("branch {b} not in exom"))
                    .with_branch(b.clone())
                    .with_suggestion(format!(
                        "ask orchestrator to run session add-agent --agent {b}"
                    ))
            }
            BranchOwned(other) => ApiError::new("branch_owned", format!("branch owned by {other}"))
                .with_suggestion("write to a branch you own, or ask orchestrator to allocate one"),
            ActorRequired => ApiError::new("actor_required", "actor required")
                .with_suggestion("pass --actor <name>"),
            Io(e) => ApiError::new("io", e.to_string()),
        }
    }
}

impl From<crate::scaffold::ScaffoldError> for ApiError {
    fn from(e: crate::scaffold::ScaffoldError) -> Self {
        use crate::scaffold::ScaffoldError::*;
        match e {
            Path(p) => ApiError::new("bad_path", p.to_string()),
            Io(e) => ApiError::new("io", e.to_string()),
            NestInsideExom(msg) => ApiError::new("cannot_nest_inside_exom", msg),
            AlreadyExistsDifferent(_, msg) => ApiError::new("already_exists_different", msg),
            NotFound(msg) => ApiError::new("not_found", msg).with_status(404),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::WriteError;

    #[test]
    fn write_error_maps_to_actor_required() {
        let api: ApiError = WriteError::ActorRequired.into();
        assert_eq!(api.code, "actor_required");
        assert!(api.suggestion.is_some());
    }

    #[test]
    fn write_error_maps_to_no_such_exom() {
        let api: ApiError = WriteError::NoSuchExom("work::ath".into()).into();
        assert_eq!(api.code, "no_such_exom");
        assert_eq!(api.path.as_deref(), Some("work::ath"));
        assert!(api.suggestion.unwrap().contains("init"));
    }
}
