use std::convert::Infallible;

use alternate_authentication::{ContextResolverError, CredentialVerifierError};
use alternate_middleware::authentication::{Authenticated, AuthenticationError};
use axum::{
    extract::FromRequestParts,
    http::{HeaderValue, header, request::Parts},
    response::{IntoResponse, Response},
};

use crate::ApiError;

#[derive(Debug, Clone)]
pub struct Auth<C>(pub C);

impl<C, S> FromRequestParts<S> for Auth<C>
where
    C: Clone + Send + Sync + 'static,
    S: Sync,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Authenticated<C>>()
            .map(|authenticated| Self(authenticated.0.clone()))
            .ok_or(AuthRejection)
    }
}

#[derive(Debug, Clone)]
pub struct MaybeAuth<C>(pub Option<C>);

impl<C, S> FromRequestParts<S> for MaybeAuth<C>
where
    C: Clone + Send + Sync + 'static,
    S: Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let context = parts
            .extensions
            .get::<Authenticated<C>>()
            .map(|authenticated| authenticated.0.clone());

        Ok(Self(context))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AuthRejection;

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        unauthenticated_response()
    }
}

#[allow(clippy::unused_async)]
pub async fn handle_authentication_error(error: AuthenticationError<Infallible>) -> Response {
    match error {
        AuthenticationError::Unauthenticated | AuthenticationError::CredentialSource(_) => {
            unauthenticated_response()
        }
        AuthenticationError::CredentialVerifier(error) => match error {
            CredentialVerifierError::MissingCredential
            | CredentialVerifierError::InvalidCredential => unauthenticated_response(),
            CredentialVerifierError::Backend(e) => ApiError::server(e).into_response(),
        },
        AuthenticationError::ContextResolver(error) => match error {
            ContextResolverError::AccessDenied => ApiError::unauthorized().into_response(),
            ContextResolverError::Backend(e) => ApiError::server(e).into_response(),
        },
        AuthenticationError::Inner(error) => match error {},
    }
}

fn unauthenticated_response() -> Response {
    let mut response = ApiError::unauthenticated().into_response();
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));

    response
}

#[cfg(test)]
mod tests {
    use alternate_middleware::authentication::AuthenticationBackendError;
    use axum::http::StatusCode;

    use super::*;

    #[tokio::test]
    async fn maps_missing_and_invalid_credentials_to_unauthenticated() {
        let errors = [
            AuthenticationError::Unauthenticated,
            AuthenticationError::CredentialVerifier(CredentialVerifierError::MissingCredential),
            AuthenticationError::CredentialVerifier(CredentialVerifierError::InvalidCredential),
        ];

        for error in errors {
            let response = handle_authentication_error(error).await;

            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(
                response.headers().get(header::WWW_AUTHENTICATE),
                Some(&HeaderValue::from_static("Bearer"))
            );
        }
    }

    #[tokio::test]
    async fn maps_resolver_denial_to_forbidden() {
        let error = AuthenticationError::ContextResolver(ContextResolverError::AccessDenied);

        let response = handle_authentication_error(error).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn maps_backend_failures_to_server_errors() {
        let errors = [
            AuthenticationError::CredentialVerifier(CredentialVerifierError::Backend(
                AuthenticationBackendError::from(anyhow::anyhow!("verifier unavailable")),
            )),
            AuthenticationError::ContextResolver(ContextResolverError::Backend(
                AuthenticationBackendError::from(anyhow::anyhow!("resolver unavailable")),
            )),
        ];

        for error in errors {
            let response = handle_authentication_error(error).await;

            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
}
