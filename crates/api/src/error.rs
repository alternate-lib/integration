use std::sync::Arc;

use axum::{
    extract::rejection::{JsonRejection, PathRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use axum_extra::extract::QueryRejection;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiErrorCode {
    BadRequest,
    Unauthenticated,
    Unauthorized,
    NotFound,
    Conflict,
    UnsupportedMediaType,
    Validation,
    BadGateway,
    Server,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
    #[serde(skip)]
    pub source: Arc<anyhow::Error>,
}

impl ApiError {
    pub fn bad_request<E: Into<anyhow::Error>>(e: E) -> Self {
        ApiError::base(ApiErrorCode::NotFound, e)
    }

    pub fn unauthenticated() -> Self {
        ApiError::base(
            ApiErrorCode::Unauthenticated,
            anyhow::anyhow!("user not authenticated"),
        )
    }

    pub fn unauthorized() -> Self {
        ApiError::base(
            ApiErrorCode::Unauthorized,
            anyhow::anyhow!("user not authorized"),
        )
    }

    pub fn not_found<E: Into<anyhow::Error>>(e: E) -> Self {
        ApiError::base(ApiErrorCode::NotFound, e)
    }

    pub fn conflict<E: Into<anyhow::Error>>(e: E) -> Self {
        ApiError::base(ApiErrorCode::Conflict, e)
    }

    pub fn validation<E: Into<anyhow::Error>>(e: E) -> Self {
        ApiError::base(ApiErrorCode::Validation, e)
    }

    pub fn server<E: Into<anyhow::Error>>(e: E) -> Self {
        ApiError::base(ApiErrorCode::Server, e)
    }

    pub fn bad_gateway<E: Into<anyhow::Error>>(e: E) -> Self {
        ApiError::base(ApiErrorCode::BadGateway, e)
    }

    fn base<E: Into<anyhow::Error>>(code: ApiErrorCode, e: E) -> Self {
        let e = e.into();

        Self {
            code,
            message: e.to_string(),
            source: Arc::new(e),
        }
    }
}

impl From<PathRejection> for ApiError {
    fn from(value: PathRejection) -> Self {
        let code = match value.status() {
            StatusCode::BAD_REQUEST => ApiErrorCode::BadRequest,
            _ => ApiErrorCode::Server,
        };

        Self::base(code, anyhow::anyhow!(value.body_text()))
    }
}

impl From<QueryRejection> for ApiError {
    fn from(value: QueryRejection) -> Self {
        let code = match value.status() {
            StatusCode::BAD_REQUEST => ApiErrorCode::BadRequest,
            _ => ApiErrorCode::Server,
        };

        Self::base(code, anyhow::anyhow!(value.body_text()))
    }
}

impl From<JsonRejection> for ApiError {
    fn from(value: JsonRejection) -> Self {
        let code = match value.status() {
            StatusCode::BAD_REQUEST => ApiErrorCode::BadRequest,
            StatusCode::UNPROCESSABLE_ENTITY => ApiErrorCode::Validation,
            StatusCode::UNSUPPORTED_MEDIA_TYPE => ApiErrorCode::UnsupportedMediaType,
            _ => ApiErrorCode::Server,
        };

        Self::base(code, anyhow::anyhow!(value.body_text()))
    }
}

impl IntoResponse for ApiError {
    fn into_response(mut self) -> Response {
        match self.code {
            ApiErrorCode::BadRequest => (StatusCode::BAD_REQUEST, axum::Json(self)).into_response(),
            ApiErrorCode::Unauthenticated => {
                (StatusCode::UNAUTHORIZED, axum::Json(self)).into_response()
            }
            ApiErrorCode::Unauthorized => (StatusCode::FORBIDDEN, axum::Json(self)).into_response(),
            ApiErrorCode::NotFound => (StatusCode::NOT_FOUND, axum::Json(self)).into_response(),
            ApiErrorCode::Conflict => (StatusCode::CONFLICT, axum::Json(self)).into_response(),
            ApiErrorCode::UnsupportedMediaType => {
                (StatusCode::UNSUPPORTED_MEDIA_TYPE, axum::Json(self)).into_response()
            }
            ApiErrorCode::Validation => {
                (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(self)).into_response()
            }
            ApiErrorCode::BadGateway => {
                let original = self.clone();

                let mut response = (StatusCode::BAD_GATEWAY, axum::Json(self)).into_response();
                response.extensions_mut().insert(original);
                response
            }
            ApiErrorCode::Server => {
                let original = self.clone();
                "unknown error".clone_into(&mut self.message);

                let mut response =
                    (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(self)).into_response();
                response.extensions_mut().insert(original);
                response
            }
        }
    }
}
