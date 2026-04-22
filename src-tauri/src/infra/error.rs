use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Unsupported format: {format}")]
    UnsupportedFormat { format: String },

    #[error("Invalid options: {message}")]
    InvalidOptions { message: String },

    #[error("No route found for {input} -> {output}; candidates: {candidates:?}")]
    NoRoute {
        input: String,
        output: String,
        candidates: Vec<String>,
    },

    #[error("File too large: {size} bytes exceeds limit {limit} bytes")]
    FileTooLarge { size: u64, limit: u64 },

    #[error("Rate limited: {message}")]
    RateLimited { message: String },

    #[error("Plugin failed: plugin={plugin_id}, step={step_index}, detail={detail}")]
    PluginFailed {
        plugin_id: String,
        step_index: usize,
        detail: String,
    },

    #[error("Task timeout: task_id={task_id}")]
    Timeout { task_id: String },

    #[error("Task not found: {task_id}")]
    TaskNotFound { task_id: String },

    #[error("Plugin not found: {plugin_id}")]
    PluginNotFound { plugin_id: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML error: {0}")]
    Toml(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ErrorResponse {
    pub error_code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl AppError {
    pub fn error_code(&self) -> &'static str {
        match self {
            AppError::UnsupportedFormat { .. } => "UNSUPPORTED_FORMAT",
            AppError::InvalidOptions { .. } => "INVALID_OPTIONS",
            AppError::NoRoute { .. } => "NO_ROUTE",
            AppError::FileTooLarge { .. } => "FILE_TOO_LARGE",
            AppError::RateLimited { .. } => "RATE_LIMITED",
            AppError::PluginFailed { .. } => "PLUGIN_FAILED",
            AppError::Timeout { .. } => "TIMEOUT",
            AppError::TaskNotFound { .. } => "TASK_NOT_FOUND",
            AppError::PluginNotFound { .. } => "PLUGIN_NOT_FOUND",
            AppError::Io(_) => "IO_ERROR",
            AppError::Json(_) => "JSON_ERROR",
            AppError::Toml(_) => "TOML_ERROR",
            AppError::Internal(_) => "INTERNAL_ERROR",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            AppError::UnsupportedFormat { .. } => 415,
            AppError::InvalidOptions { .. } => 422,
            AppError::NoRoute { .. } => 422,
            AppError::FileTooLarge { .. } => 413,
            AppError::RateLimited { .. } => 429,
            AppError::PluginFailed { .. } => 500,
            AppError::Timeout { .. } => 504,
            AppError::TaskNotFound { .. } => 404,
            AppError::PluginNotFound { .. } => 404,
            _ => 500,
        }
    }

    pub fn to_response(&self) -> ErrorResponse {
        ErrorResponse {
            error_code: self.error_code().to_string(),
            message: self.to_string(),
            details: self.details(),
        }
    }

    fn details(&self) -> Option<serde_json::Value> {
        match self {
            AppError::NoRoute { candidates, .. } => Some(serde_json::json!({
                "candidates": candidates
            })),
            AppError::PluginFailed {
                plugin_id,
                step_index,
                ..
            } => Some(serde_json::json!({
                "plugin_id": plugin_id,
                "step_index": step_index
            })),
            _ => None,
        }
    }
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        let status =
            StatusCode::from_u16(self.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::Json(self.to_response());
        (status, body).into_response()
    }
}
