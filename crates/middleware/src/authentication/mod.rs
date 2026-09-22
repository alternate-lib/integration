use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use alternate_authentication::{
    ContextResolver, ContextResolverError, CredentialVerifier, CredentialVerifierError,
};
use credential_source::{CredentialSourceError, CredentialSources};
use http::Request;
use tower::{Layer, Service};

mod credential_source;

#[derive(Debug, Clone)]
pub struct Authenticated<C>(pub C);

#[derive(Debug, Clone)]
pub struct AuthenticationLayer<SS, CV, CR> {
    mode: AuthenticationMode,
    credential_sources: CredentialSources,
    scope_selector: Arc<SS>,
    credential_verifier: Arc<CV>,
    context_resolver: Arc<CR>,
}

impl<SS, CV, CR> AuthenticationLayer<SS, CV, CR> {
    pub fn new(
        scope_selector: SS,
        mode: AuthenticationMode,
        credential_verifier: CV,
        context_resolver: CR,
    ) -> Self {
        Self {
            mode,
            credential_sources: CredentialSources::default(),
            scope_selector: Arc::new(scope_selector),
            credential_verifier: Arc::new(credential_verifier),
            context_resolver: Arc::new(context_resolver),
        }
    }

    #[must_use]
    pub fn with_credential_sources(mut self, credential_sources: CredentialSources) -> Self {
        self.credential_sources = credential_sources;
        self
    }
}

impl<S, SS, CV, CR> Layer<S> for AuthenticationLayer<SS, CV, CR> {
    type Service = AuthenticationService<S, SS, CV, CR>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthenticationService {
            inner,
            mode: self.mode,
            credential_sources: self.credential_sources.clone(),
            scope_selector: self.scope_selector.clone(),
            context_resolver: self.context_resolver.clone(),
            credential_verifier: self.credential_verifier.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthenticationService<S, SS, CV, CR> {
    inner: S,
    mode: AuthenticationMode,
    credential_sources: CredentialSources,
    scope_selector: Arc<SS>,
    credential_verifier: Arc<CV>,
    context_resolver: Arc<CR>,
}

impl<B, S, SS, CV, CR> Service<Request<B>> for AuthenticationService<S, SS, CV, CR>
where
    B: Send + 'static,
    S: Service<Request<B>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    SS: ScopeSelector + Send + Sync + 'static,
    CV: CredentialVerifier + Send + Sync + 'static,
    CV::BackendError: Send + Sync,
    CR: ContextResolver<Evidence = CV::Evidence, Scope = SS::Scope> + Send + Sync + 'static,
    CR::Context: Clone + Send + Sync + 'static,
    CR::BackendError: Send + Sync,
{
    type Response = S::Response;
    type Error = AuthenticationError<S::Error>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        let mut inner = self.inner.clone();
        let mode = self.mode;
        let credential_sources = self.credential_sources.clone();
        let scope_selector = self.scope_selector.clone();
        let context_resolver = self.context_resolver.clone();
        let credential_verifier = self.credential_verifier.clone();

        Box::pin(async move {
            let (parts, body) = request.into_parts();
            let credential = credential_sources.extract(&parts)?;
            let mut request = Request::from_parts(parts, body);

            let Some(credential) = credential else {
                return match mode {
                    AuthenticationMode::Required => Err(AuthenticationError::Unauthenticated),
                    AuthenticationMode::Optional => inner
                        .call(request)
                        .await
                        .map_err(AuthenticationError::Inner),
                };
            };

            let verified_credential = credential_verifier
                .verify(credential)
                .await
                .map_err(|e| e.map_backend(|e| AuthenticationBackendError(e.into())))?;
            let scope = scope_selector.select(&request)?;
            let context = context_resolver
                .resolve(verified_credential, scope)
                .await
                .map_err(|e| e.map_backend(|e| AuthenticationBackendError(e.into())))?;

            request.extensions_mut().insert(Authenticated(context));

            inner
                .call(request)
                .await
                .map_err(AuthenticationError::Inner)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationMode {
    Required,
    Optional,
}

pub trait ScopeSelector {
    type Scope;

    fn select<B>(
        &self,
        request: &Request<B>,
    ) -> Result<Self::Scope, ContextResolverError<AuthenticationBackendError>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoScope;

impl ScopeSelector for NoScope {
    type Scope = ();

    fn select<B>(
        &self,
        _request: &Request<B>,
    ) -> Result<Self::Scope, ContextResolverError<AuthenticationBackendError>> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct FixedScope<S>(pub S);

impl<S: Clone> ScopeSelector for FixedScope<S> {
    type Scope = S;

    fn select<B>(
        &self,
        _request: &Request<B>,
    ) -> Result<Self::Scope, ContextResolverError<AuthenticationBackendError>> {
        Ok(self.0.clone())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthenticationError<E> {
    #[error("request not authenticated")]
    Unauthenticated,

    #[error(transparent)]
    CredentialSource(#[from] CredentialSourceError),

    #[error("credential verifier: {0}")]
    CredentialVerifier(#[from] CredentialVerifierError<AuthenticationBackendError>),

    #[error("context resolver: {0}")]
    ContextResolver(#[from] ContextResolverError<AuthenticationBackendError>),

    #[error(transparent)]
    Inner(E),
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct AuthenticationBackendError(#[from] anyhow::Error);

#[cfg(test)]
mod tests {
    use std::{assert_matches, convert::Infallible};

    use alternate_authentication::{
        AuthenticatedIdentity, Credential, CredentialContext, CredentialRestrictions, Issuer,
        Subject, SubjectId, VerifiedCredential,
    };
    use http::header;
    use tower::{ServiceExt as _, service_fn};

    use super::*;

    #[derive(Clone, Copy)]
    struct AcceptVerifier;

    impl CredentialVerifier for AcceptVerifier {
        type Evidence = ();
        type BackendError = AuthenticationBackendError;

        async fn verify(
            &self,
            credential: Credential,
        ) -> Result<VerifiedCredential<Self::Evidence>, CredentialVerifierError<Self::BackendError>>
        {
            Ok(VerifiedCredential::new(identity_for(&credential), ()))
        }
    }

    fn identity_for(credential: &Credential) -> AuthenticatedIdentity {
        AuthenticatedIdentity::new(
            Subject::new(
                Issuer::try_new("identity").unwrap(),
                SubjectId::try_new(credential.expose_secret()).unwrap(),
            ),
            CredentialContext::new(
                credential.kind(),
                None,
                CredentialRestrictions::unrestricted(),
            ),
        )
    }

    #[derive(Clone, Copy)]
    struct SubjectResolver;

    impl ContextResolver for SubjectResolver {
        type Evidence = ();
        type Scope = ();
        type Context = String;
        type BackendError = AuthenticationBackendError;

        async fn resolve(
            &self,
            credential: VerifiedCredential<Self::Evidence>,
            (): Self::Scope,
        ) -> Result<Self::Context, ContextResolverError<Self::BackendError>> {
            Ok(credential.identity().subject().id().to_string())
        }
    }

    #[derive(Clone, Copy)]
    struct InvalidVerifier;

    impl CredentialVerifier for InvalidVerifier {
        type Evidence = ();
        type BackendError = AuthenticationBackendError;

        async fn verify(
            &self,
            _credential: Credential,
        ) -> Result<VerifiedCredential<Self::Evidence>, CredentialVerifierError<Self::BackendError>>
        {
            Err(CredentialVerifierError::InvalidCredential)
        }
    }

    #[derive(Clone, Copy)]
    struct BackendFailingVerifier;

    impl CredentialVerifier for BackendFailingVerifier {
        type Evidence = ();
        type BackendError = AuthenticationBackendError;

        async fn verify(
            &self,
            _credential: Credential,
        ) -> Result<VerifiedCredential<Self::Evidence>, CredentialVerifierError<Self::BackendError>>
        {
            Err(CredentialVerifierError::Backend(
                anyhow::anyhow!("verifier unavailable").into(),
            ))
        }
    }

    #[derive(Clone, Copy)]
    struct DenyingResolver;

    impl ContextResolver for DenyingResolver {
        type Evidence = ();
        type Scope = ();
        type Context = String;
        type BackendError = AuthenticationBackendError;

        async fn resolve(
            &self,
            _credential: VerifiedCredential<Self::Evidence>,
            (): Self::Scope,
        ) -> Result<Self::Context, ContextResolverError<Self::BackendError>> {
            Err(ContextResolverError::AccessDenied)
        }
    }

    #[derive(Clone, Copy)]
    struct BackendFailingResolver;

    impl ContextResolver for BackendFailingResolver {
        type Evidence = ();
        type Scope = ();
        type Context = String;
        type BackendError = AuthenticationBackendError;

        async fn resolve(
            &self,
            _credential: VerifiedCredential<Self::Evidence>,
            (): Self::Scope,
        ) -> Result<Self::Context, ContextResolverError<Self::BackendError>> {
            Err(ContextResolverError::Backend(
                anyhow::anyhow!("resolver unavailable").into(),
            ))
        }
    }

    struct Sentinel(&'static str);

    #[derive(Clone, Copy)]
    struct EvidenceVerifier;

    impl CredentialVerifier for EvidenceVerifier {
        type Evidence = Sentinel;
        type BackendError = AuthenticationBackendError;

        async fn verify(
            &self,
            credential: Credential,
        ) -> Result<VerifiedCredential<Self::Evidence>, CredentialVerifierError<Self::BackendError>>
        {
            Ok(VerifiedCredential::new(
                identity_for(&credential),
                Sentinel("verifier-produced"),
            ))
        }
    }

    #[derive(Clone, Copy)]
    struct EvidenceResolver;

    impl ContextResolver for EvidenceResolver {
        type Evidence = Sentinel;
        type Scope = ();
        type Context = &'static str;
        type BackendError = AuthenticationBackendError;

        async fn resolve(
            &self,
            credential: VerifiedCredential<Self::Evidence>,
            (): Self::Scope,
        ) -> Result<Self::Context, ContextResolverError<Self::BackendError>> {
            let (_, evidence) = credential.into_parts();
            Ok(evidence.0)
        }
    }

    #[tokio::test]
    async fn authenticates_and_inserts_resolved_context() {
        let inner = service_fn(|request: Request<()>| async move {
            let context = request.extensions().get::<Authenticated<String>>().unwrap();
            Ok::<_, Infallible>(context.0.clone())
        });
        let service = AuthenticationLayer::new(
            NoScope,
            AuthenticationMode::Required,
            AcceptVerifier,
            SubjectResolver,
        )
        .layer(inner);
        let request = Request::builder()
            .header(header::AUTHORIZATION, "Bearer subject-1")
            .body(())
            .unwrap();

        let response = service.oneshot(request).await.unwrap();

        assert_eq!(response, "subject-1");
    }

    #[tokio::test]
    async fn optional_auth_allows_anonymous_requests() {
        let inner = service_fn(|request: Request<()>| async move {
            Ok::<_, Infallible>(
                request
                    .extensions()
                    .get::<Authenticated<String>>()
                    .is_some(),
            )
        });
        let service = AuthenticationLayer::new(
            NoScope,
            AuthenticationMode::Optional,
            AcceptVerifier,
            SubjectResolver,
        )
        .layer(inner);

        let authenticated = service
            .oneshot(Request::new(()))
            .await
            .expect("anonymous request should reach the inner service");

        assert!(!authenticated);
    }

    #[tokio::test]
    async fn required_auth_rejects_missing_credentials() {
        let service = AuthenticationLayer::new(
            NoScope,
            AuthenticationMode::Required,
            AcceptVerifier,
            SubjectResolver,
        )
        .layer(service_fn(|_| async move { Ok::<_, Infallible>(()) }));
        let result = service.oneshot(Request::new(())).await;

        assert_matches!(result, Err(AuthenticationError::Unauthenticated));
    }

    #[tokio::test]
    async fn optional_auth_rejects_malformed_supplied_credentials() {
        let service = AuthenticationLayer::new(
            NoScope,
            AuthenticationMode::Optional,
            AcceptVerifier,
            SubjectResolver,
        )
        .layer(service_fn(|_| async move { Ok::<_, Infallible>(()) }));
        let request = Request::builder()
            .header(header::AUTHORIZATION, "Basic token")
            .body(())
            .unwrap();

        let result = service.oneshot(request).await;

        assert_matches!(
            result,
            Err(AuthenticationError::CredentialSource(
                CredentialSourceError::Malformed
            ))
        );
    }

    #[tokio::test]
    async fn optional_auth_rejects_invalid_supplied_credentials() {
        let service = AuthenticationLayer::new(
            NoScope,
            AuthenticationMode::Optional,
            InvalidVerifier,
            SubjectResolver,
        )
        .layer(service_fn(|_| async move { Ok::<_, Infallible>(()) }));
        let request = Request::builder()
            .header(header::AUTHORIZATION, "Bearer invalid")
            .body(())
            .unwrap();

        let result = service.oneshot(request).await;

        assert_matches!(
            result,
            Err(AuthenticationError::CredentialVerifier(
                CredentialVerifierError::InvalidCredential
            ))
        );
    }

    #[tokio::test]
    async fn propagates_verifier_backend_failures() {
        let service = AuthenticationLayer::new(
            NoScope,
            AuthenticationMode::Required,
            BackendFailingVerifier,
            SubjectResolver,
        )
        .layer(service_fn(|_| async move { Ok::<_, Infallible>(()) }));
        let request = Request::builder()
            .header(header::AUTHORIZATION, "Bearer subject-1")
            .body(())
            .unwrap();

        let result = service.oneshot(request).await;

        assert_matches!(
            result,
            Err(AuthenticationError::CredentialVerifier(
                CredentialVerifierError::Backend(_)
            ))
        );
    }

    #[tokio::test]
    async fn propagates_resolver_access_denial() {
        let service = AuthenticationLayer::new(
            NoScope,
            AuthenticationMode::Required,
            AcceptVerifier,
            DenyingResolver,
        )
        .layer(service_fn(|_| async move { Ok::<_, Infallible>(()) }));
        let request = Request::builder()
            .header(header::AUTHORIZATION, "Bearer subject-1")
            .body(())
            .unwrap();

        let result = service.oneshot(request).await;

        assert_matches!(
            result,
            Err(AuthenticationError::ContextResolver(
                ContextResolverError::AccessDenied
            ))
        );
    }

    #[tokio::test]
    async fn propagates_resolver_backend_failures() {
        let service = AuthenticationLayer::new(
            NoScope,
            AuthenticationMode::Required,
            AcceptVerifier,
            BackendFailingResolver,
        )
        .layer(service_fn(|_| async move { Ok::<_, Infallible>(()) }));
        let request = Request::builder()
            .header(header::AUTHORIZATION, "Bearer subject-1")
            .body(())
            .unwrap();

        let result = service.oneshot(request).await;

        assert_matches!(
            result,
            Err(AuthenticationError::ContextResolver(
                ContextResolverError::Backend(_)
            ))
        );
    }

    #[tokio::test]
    async fn transports_non_unit_evidence_without_inspection() {
        let inner = service_fn(|request: Request<()>| async move {
            let context = request
                .extensions()
                .get::<Authenticated<&'static str>>()
                .unwrap();
            Ok::<_, Infallible>(context.0)
        });
        let service = AuthenticationLayer::new(
            NoScope,
            AuthenticationMode::Required,
            EvidenceVerifier,
            EvidenceResolver,
        )
        .layer(inner);
        let request = Request::builder()
            .header(header::AUTHORIZATION, "Bearer subject-1")
            .body(())
            .unwrap();

        let response = service.oneshot(request).await.unwrap();

        assert_eq!(response, "verifier-produced");
    }
}
