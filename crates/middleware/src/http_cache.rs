use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use alternate_codec::Codec;
use alternate_http_cache::{CacheDecision, HttpCache, HttpCacheConfig, RevalidationOutcome};
use alternate_platform::kv::KvExpiryTyped;
use bytes::Bytes;
use http::{Method, Request, Response};
use tower::{Layer, Service, ServiceExt as _};

#[derive(Clone)]
pub struct HttpCacheLayer<C: Codec, KET: KvExpiryTyped<C>> {
    cache: Arc<HttpCache<C, KET>>,
}

impl<C: Codec, KET: KvExpiryTyped<C> + Sync> HttpCacheLayer<C, KET> {
    pub fn new(kv_client: KET) -> Self {
        Self::with_config(kv_client, HttpCacheConfig::builder().build())
    }

    pub fn with_config(kv_client: KET, config: HttpCacheConfig) -> Self {
        Self {
            cache: Arc::new(HttpCache::new(kv_client).with_config(config)),
        }
    }

    pub fn from_cache(cache: Arc<HttpCache<C, KET>>) -> Self {
        Self { cache }
    }
}

impl<C, KET, S> Layer<S> for HttpCacheLayer<C, KET>
where
    C: Codec,
    KET: KvExpiryTyped<C>,
{
    type Service = HttpCacheService<C, KET, S>;

    fn layer(&self, inner: S) -> Self::Service {
        HttpCacheService {
            cache: self.cache.clone(),
            inner,
        }
    }
}

#[derive(Clone)]
pub struct HttpCacheService<C: Codec, KET: KvExpiryTyped<C>, S> {
    cache: Arc<HttpCache<C, KET>>,
    inner: S,
}

impl<C, KET, S> Service<Request<Bytes>> for HttpCacheService<C, KET, S>
where
    C: Codec + Send + Sync + 'static,
    C::Error: Send,
    KET: KvExpiryTyped<C> + Send + Sync + 'static,
    KET::Error: Send,
    S: Service<Request<Bytes>, Response = Response<Bytes>> + Clone + Send + 'static,
    S::Future: Send,
    S::Error: Send,
{
    type Response = Response<Bytes>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Bytes>) -> Self::Future {
        let cache = self.cache.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let decision = cache.lookup(&req).await;
            match decision {
                Ok(CacheDecision::Fresh(response)) => return Ok(response),
                Ok(CacheDecision::Revalidate(revalidation)) => {
                    let response = inner.ready().await?.call(revalidation.request).await?;

                    if let RevalidationOutcome::Response(resp) =
                        cache.revalidated(&req, revalidation.cached, response).await
                    {
                        return Ok(resp);
                    }
                }
                Ok(CacheDecision::Bypass | CacheDecision::Miss) | Err(_) => {}
            }

            let response = inner.oneshot(req.clone()).await?;
            if can_mutate(req.method())
                && (response.status().is_success() || response.status().is_redirection())
            {
                cache.invalidate(&req).await;
            } else {
                let _ = cache.store(&req, &response).await;
            }

            Ok(response)
        })
    }
}

fn can_mutate(method: &Method) -> bool {
    !matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}
