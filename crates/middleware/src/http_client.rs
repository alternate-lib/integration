use std::{
    pin::Pin,
    task::{Context, Poll},
};

use alternate_http::HttpClient;
use bytes::Bytes;
use http::{Request, Response};
use tower::{Service, ServiceExt as _, buffer::Buffer};

#[derive(Debug, Clone)]
pub struct HttpClientService<HC: HttpClient> {
    client: HC,
}

impl<HC: HttpClient> HttpClientService<HC> {
    pub fn new(client: HC) -> Self {
        Self { client }
    }
}

impl<HC: HttpClient + Clone + 'static> Service<Request<Bytes>> for HttpClientService<HC> {
    type Response = Response<Bytes>;
    type Error = HC::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Bytes>) -> Self::Future {
        let client = self.client.clone();

        Box::pin(async move { client.send(req).await })
    }
}

#[derive(Clone)]
pub struct TowerHttpClient {
    #[allow(clippy::type_complexity)]
    inner: Buffer<
        Request<Bytes>,
        Pin<Box<dyn Future<Output = Result<Response<Bytes>, tower::BoxError>> + Send>>,
    >,
}

impl TowerHttpClient {
    pub fn new<S>(service: S, bound: usize) -> Self
    where
        S: Service<Request<Bytes>, Response = Response<Bytes>> + Send + 'static,
        S::Future: Send + 'static,
        S::Error: Into<tower::BoxError> + Send + Sync,
    {
        Self {
            inner: Buffer::new(
                service.map_err(Into::<tower::BoxError>::into).boxed(),
                bound,
            ),
        }
    }
}

impl HttpClient for TowerHttpClient {
    type Error = HttpClientServiceError;

    async fn send(&self, request: Request<Bytes>) -> Result<Response<Bytes>, Self::Error> {
        self.inner
            .clone()
            .ready()
            .await?
            .call(request)
            .await
            .map_err(HttpClientServiceError)
    }
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct HttpClientServiceError(#[from] tower::BoxError);
