use std::{fmt, sync::Arc};

use alternate_authentication::Credential;
use headers::{Authorization, HeaderMapExt, authorization::Bearer};
use http::{HeaderMap, HeaderName, HeaderValue, request::Parts};

#[derive(Clone)]
pub struct CredentialSources {
    sources: Vec<Arc<dyn CredentialSource + Send + Sync>>,
}

impl CredentialSources {
    pub const fn new() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    #[must_use]
    pub fn with<S>(mut self, source: S) -> Self
    where
        S: CredentialSource + Send + Sync + 'static,
    {
        self.sources.push(Arc::new(source));
        self
    }

    pub(super) fn extract(
        &self,
        parts: &Parts,
    ) -> Result<Option<Credential>, CredentialSourceError> {
        let mut credential = None;
        let mut unsupported = false;

        for source in &self.sources {
            match source.extract(parts)? {
                CredentialSourceOutcome::Absent => {}
                CredentialSourceOutcome::Unsupported => unsupported = true,
                CredentialSourceOutcome::Credential(candidate) => {
                    if credential.replace(candidate).is_some() {
                        return Err(CredentialSourceError::Ambiguous);
                    }
                }
            }
        }

        if credential.is_none() && unsupported {
            return Err(CredentialSourceError::Unsupported);
        }

        Ok(credential)
    }
}

impl fmt::Debug for CredentialSources {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialSources")
            .field("source_count", &self.sources.len())
            .finish()
    }
}

impl Default for CredentialSources {
    fn default() -> Self {
        Self::new()
            .with(BearerHeader)
            .with(ApiKeyHeader::new(HeaderName::from_static("x-api-key")))
    }
}

pub trait CredentialSource {
    fn extract(&self, parts: &Parts) -> Result<CredentialSourceOutcome, CredentialSourceError>;
}

#[derive(Debug)]
pub enum CredentialSourceOutcome {
    Credential(Credential),
    Absent,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BearerHeader;

impl CredentialSource for BearerHeader {
    fn extract(&self, parts: &Parts) -> Result<CredentialSourceOutcome, CredentialSourceError> {
        let token = match parts.headers.typed_try_get::<Authorization<Bearer>>() {
            Ok(Some(header)) => header.token().to_owned(),
            Ok(None) => return Ok(CredentialSourceOutcome::Absent),
            Err(_) => return Err(CredentialSourceError::Malformed),
        };

        if token.is_empty() || token.contains(char::is_whitespace) {
            return Err(CredentialSourceError::Malformed);
        }

        Credential::bearer(token)
            .map(CredentialSourceOutcome::Credential)
            .map_err(|_| CredentialSourceError::Malformed)
    }
}

#[derive(Debug, Clone)]
pub struct ApiKeyHeader {
    header: HeaderName,
}

impl ApiKeyHeader {
    pub fn new(header: HeaderName) -> Self {
        Self { header }
    }
}

impl CredentialSource for ApiKeyHeader {
    fn extract(&self, parts: &Parts) -> Result<CredentialSourceOutcome, CredentialSourceError> {
        let Some(value) = one_header(&parts.headers, &self.header)? else {
            return Ok(CredentialSourceOutcome::Absent);
        };
        let value = value
            .to_str()
            .map_err(|_| CredentialSourceError::Malformed)?;

        Credential::api_key(value)
            .map(CredentialSourceOutcome::Credential)
            .map_err(|_| CredentialSourceError::Malformed)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialSourceError {
    #[error("credential source contains multiple values")]
    Duplicate,

    #[error("credential source is malformed")]
    Malformed,

    #[error("credential source is not supported")]
    Unsupported,

    #[error("multiple credential sources were supplied")]
    Ambiguous,
}

fn one_header<'a>(
    headers: &'a HeaderMap,
    name: &HeaderName,
) -> Result<Option<&'a HeaderValue>, CredentialSourceError> {
    let mut values = headers.get_all(name).iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(CredentialSourceError::Duplicate);
    }

    Ok(first)
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use http::{Request, header};

    use super::*;

    #[derive(Debug)]
    struct CustomSource;

    impl CredentialSource for CustomSource {
        fn extract(
            &self,
            _parts: &Parts,
        ) -> Result<CredentialSourceOutcome, CredentialSourceError> {
            Ok(CredentialSourceOutcome::Credential(
                Credential::api_key("custom").unwrap(),
            ))
        }
    }

    #[derive(Debug)]
    struct BasicHeader;

    impl CredentialSource for BasicHeader {
        fn extract(&self, parts: &Parts) -> Result<CredentialSourceOutcome, CredentialSourceError> {
            let Some(value) = one_header(&parts.headers, &header::AUTHORIZATION)? else {
                return Ok(CredentialSourceOutcome::Absent);
            };
            let value = value
                .to_str()
                .map_err(|_| CredentialSourceError::Malformed)?;
            let (scheme, token) = value
                .split_once(' ')
                .ok_or(CredentialSourceError::Malformed)?;
            if !scheme.eq_ignore_ascii_case("basic") {
                return Ok(CredentialSourceOutcome::Unsupported);
            }

            Credential::api_key(token)
                .map(CredentialSourceOutcome::Credential)
                .map_err(|_| CredentialSourceError::Malformed)
        }
    }

    fn extract(
        headers: HeaderMap,
        sources: &CredentialSources,
    ) -> Result<Option<Credential>, CredentialSourceError> {
        let (mut parts, ()) = Request::new(()).into_parts();
        parts.headers = headers;
        sources.extract(&parts)
    }

    #[test]
    fn accepts_custom_credential_sources() {
        let sources = CredentialSources::new().with(CustomSource);

        let credential = extract(HeaderMap::new(), &sources).unwrap().unwrap();

        assert_eq!(credential.expose_secret(), "custom");
    }

    #[test]
    fn supports_sources_that_share_a_header() {
        let sources = CredentialSources::new()
            .with(BearerHeader)
            .with(BasicHeader);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer token"),
        );

        let credential = extract(headers, &sources).unwrap().unwrap();

        assert_eq!(credential.expose_secret(), "token");
    }

    #[test]
    fn parses_a_bearer_credential() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer token"),
        );

        let credential = extract(headers, &CredentialSources::default())
            .unwrap()
            .unwrap();

        assert_eq!(credential.expose_secret(), "token");
    }

    #[test]
    fn rejects_ambiguous_credentials() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer token"),
        );
        headers.insert("x-api-key", HeaderValue::from_static("api-key"));

        assert_matches!(
            extract(headers, &CredentialSources::default()),
            Err(CredentialSourceError::Ambiguous)
        );
    }

    #[test]
    fn rejects_malformed_bearer_credentials() {
        for value in ["token", "Basic token", "Bearer", "Bearer one two"] {
            let mut headers = HeaderMap::new();
            headers.insert(header::AUTHORIZATION, HeaderValue::from_str(value).unwrap());

            assert!(extract(headers, &CredentialSources::default()).is_err());
        }
    }
}
