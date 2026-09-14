use std::{marker::PhantomData, time::SystemTime};

use alternate_codec::Codec;
use alternate_crypto::digest::{Blake3, Hasher as _};
use alternate_platform::kv::{KvClientTyped, KvExpiryTyped, KvTypedError};
use bytes::Bytes;
pub use config::{HttpCacheConfig, HttpCacheConfigBuilder};
use http::{HeaderMap, Method, Request, Response, StatusCode, header};
use http_cache_semantics::{AfterResponse, BeforeRequest, CachePolicy};

mod config;

const SCHEMA_VERSION: u8 = 1;

pub struct HttpCache<C: Codec, KET: KvExpiryTyped<C>> {
    kv_client: KET,
    config: HttpCacheConfig,
    _codec: PhantomData<C>,
}

impl<C: Codec, KET: KvExpiryTyped<C> + Sync> HttpCache<C, KET> {
    pub fn new(kv_client: KET) -> Self {
        Self {
            kv_client,
            config: HttpCacheConfig::builder().build(),
            _codec: PhantomData,
        }
    }

    #[must_use]
    pub fn with_config(mut self, config: HttpCacheConfig) -> Self {
        self.config = config;
        self
    }

    pub async fn lookup(
        &self,
        req: &Request<Bytes>,
    ) -> Result<CacheDecision, HttpCacheError<KvTypedError<C::Error, KET::Error>>> {
        if !cacheable(req, &self.config) {
            return Ok(CacheDecision::Bypass);
        }

        let result = <KET as KvClientTyped<C>>::get_typed::<CacheEntry>(
            &self.kv_client,
            &cache_key(&self.config.key_prefix, req),
        )
        .await;
        let entry = match result {
            Ok(Some(entry)) => entry,
            Ok(None) => return Ok(CacheDecision::Miss),
            Err(KvTypedError::Codec(error)) => {
                #[cfg(feature = "tracing")]
                tracing::warn!(?error, "HTTP cache entry could not be decoded");
                #[cfg(not(feature = "tracing"))]
                let _ = error;

                self.invalidate(req).await;

                return Ok(CacheDecision::Miss);
            }
            Err(KvTypedError::Client(error)) => {
                #[cfg(feature = "tracing")]
                tracing::warn!(?error, "HTTP cache read failed");
                #[cfg(not(feature = "tracing"))]
                let _ = error;

                return Ok(CacheDecision::Miss);
            }
        };

        if entry.version != SCHEMA_VERSION {
            self.invalidate(req).await;

            return Ok(CacheDecision::Miss);
        }

        let (response, policy) = match entry.into_parts() {
            Ok(resp) => resp,
            Err(error) => {
                #[cfg(feature = "tracing")]
                tracing::warn!(?error, "HTTP cache entry was malformed");
                #[cfg(not(feature = "tracing"))]
                let _ = error;

                self.invalidate(req).await;

                return Ok(CacheDecision::Miss);
            }
        };

        let cached = CachedResponse { response, policy };
        let policy_req = normalized_get_request(req);

        match cached
            .policy
            .before_request(policy_req.as_ref().unwrap_or(req), SystemTime::now())
        {
            BeforeRequest::Fresh(parts) => {
                let body = if req.method() == Method::HEAD {
                    Bytes::new()
                } else {
                    cached.response.into_body()
                };
                Ok(CacheDecision::Fresh(Response::from_parts(parts, body)))
            }
            BeforeRequest::Stale {
                request: parts,
                matches: true,
            } => {
                let mut request = req.clone();
                *request.method_mut() = parts.method;
                *request.uri_mut() = parts.uri;
                *request.headers_mut() = parts.headers;

                Ok(CacheDecision::Revalidate(Box::new(Revalidation {
                    request,
                    cached,
                })))
            }
            BeforeRequest::Stale { .. } => Ok(CacheDecision::Miss),
        }
    }

    pub async fn store(
        &self,
        req: &Request<Bytes>,
        resp: &Response<Bytes>,
    ) -> Result<(), HttpCacheError<KvTypedError<C::Error, KET::Error>>> {
        self.store_inner(req, resp).await?;

        Ok(())
    }

    async fn store_inner(
        &self,
        req: &Request<Bytes>,
        resp: &Response<Bytes>,
    ) -> Result<bool, HttpCacheError<KvTypedError<C::Error, KET::Error>>> {
        if req.method() != Method::GET {
            return Ok(false);
        }

        let policy = CachePolicy::new(req, resp);

        if !policy.is_storable()
            || !cacheable(req, &self.config)
            || (!self.config.cache_set_cookie_responses
                && resp.headers().contains_key(header::SET_COOKIE))
        {
            return Ok(false);
        }

        let ttl = policy
            .time_to_live(SystemTime::now())
            .saturating_add(self.config.stale_retention);
        if ttl.is_zero() {
            return Ok(false);
        }

        let entry = CacheEntry::from_response(resp, policy);
        let encoded = C::encode(&entry).map_err(KvTypedError::Codec)?;

        if self
            .config
            .max_entry_size
            .is_some_and(|size| encoded.len() > size)
        {
            return Ok(false);
        }

        self.kv_client
            .set_with_ttl(&cache_key(&self.config.key_prefix, req), &encoded, ttl)
            .await
            .map_err(KvTypedError::Client)?;

        Ok(true)
    }

    pub async fn invalidate(&self, req: &Request<Bytes>) {
        let _ = self
            .kv_client
            .delete(&cache_key(&self.config.key_prefix, req))
            .await;
    }

    pub async fn revalidated(
        &self,
        req: &Request<Bytes>,
        cached: CachedResponse,
        mut resp: Response<Bytes>,
    ) -> RevalidationOutcome {
        let normalized = normalized_get_request(req);
        let policy_req = normalized.as_ref().unwrap_or(req);

        if resp.status() == StatusCode::NOT_MODIFIED {
            let parts = match cached
                .policy
                .after_response(policy_req, &resp, SystemTime::now())
            {
                AfterResponse::NotModified(_, parts) => parts,
                AfterResponse::Modified(..) => {
                    self.invalidate(req).await;

                    return RevalidationOutcome::Retry;
                }
            };

            resp = Response::from_parts(parts, cached.response.into_body());
        }

        if !matches!(self.store_inner(policy_req, &resp).await, Ok(true)) {
            self.invalidate(req).await;
        }

        if req.method() == Method::HEAD {
            *resp.body_mut() = Bytes::new();
        }

        RevalidationOutcome::Response(resp)
    }
}

#[derive(Debug)]
pub enum CacheDecision {
    Bypass,
    Miss,
    Fresh(Response<Bytes>),
    Revalidate(Box<Revalidation>),
}

#[derive(Debug)]
pub struct Revalidation {
    pub request: Request<Bytes>,
    pub cached: CachedResponse,
}

#[derive(Debug)]
pub struct CachedResponse {
    pub response: Response<Bytes>,
    policy: CachePolicy,
}

#[derive(Debug)]
pub enum RevalidationOutcome {
    Response(Response<Bytes>),
    Retry,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    version: u8,
    #[serde(with = "http_serde::status_code")]
    status: StatusCode,
    #[serde(with = "http_serde::header_map")]
    headers: HeaderMap,
    body: Vec<u8>,
    policy: CachePolicy,
}

impl CacheEntry {
    fn from_response(response: &Response<Bytes>, policy: CachePolicy) -> Self {
        Self {
            version: SCHEMA_VERSION,
            status: response.status(),
            headers: response.headers().to_owned(),
            body: response.body().to_vec(),
            policy,
        }
    }

    fn into_parts(self) -> Result<(Response<Bytes>, CachePolicy), http::Error> {
        let mut builder = Response::builder().status(self.status);
        if let Some(headers) = builder.headers_mut() {
            *headers = self.headers;
        }

        let resp = builder.body(self.body.into())?;

        Ok((resp, self.policy))
    }
}

fn cacheable(req: &Request<Bytes>, config: &HttpCacheConfig) -> bool {
    matches!(*req.method(), Method::GET | Method::HEAD)
        && (config.cache_authorized_requests || !req.headers().contains_key(header::AUTHORIZATION))
        && ![
            header::IF_MATCH,
            header::IF_UNMODIFIED_SINCE,
            header::IF_NONE_MATCH,
            header::IF_MODIFIED_SINCE,
            header::IF_RANGE,
            header::RANGE,
        ]
        .iter()
        .any(|name| req.headers().contains_key(name))
}

fn normalized_get_request(req: &Request<Bytes>) -> Option<Request<Bytes>> {
    if req.method() != Method::HEAD {
        return None;
    }

    let mut normalized = req.clone();
    *normalized.method_mut() = Method::GET;
    Some(normalized)
}

fn cache_key(prefix: &str, req: &Request<Bytes>) -> String {
    let mut hasher = Blake3::hasher();
    hasher
        .update(b"GET\n")
        .update(req.uri().to_string().as_bytes());

    format!("{prefix}:{}", hasher.finalize().to_hex())
}

#[derive(Debug, thiserror::Error)]
pub enum HttpCacheError<KvErr: std::error::Error> {
    #[error("kv: {0}")]
    Kv(#[from] KvErr),
}
