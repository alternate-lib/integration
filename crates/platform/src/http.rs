use alternate_codec::Codec;
use alternate_http::HttpClient;
use bytes::Bytes;
use http::{Method, Request, Response};
use serde::{Serialize, de::DeserializeOwned};

pub trait HttpClientTyped<C: Codec>: HttpClient {
    fn send_typed<U: Serialize, V: DeserializeOwned>(
        &self,
        request: Request<U>,
    ) -> impl Future<Output = Result<Response<V>, HttpTypedError<C::Error, Self::Error>>>;
}

impl<HC: HttpClient, C: Codec> HttpClientTyped<C> for HC {
    async fn send_typed<U: Serialize, V: DeserializeOwned>(
        &self,
        request: Request<U>,
    ) -> Result<Response<V>, HttpTypedError<C::Error, Self::Error>> {
        let (parts, data) = request.into_parts();
        let raw = C::encode(&data).map_err(HttpTypedError::Codec)?;
        let request = Request::from_parts(parts, raw.into());

        let resp = self.send(request).await?;

        let (parts, raw) = resp.into_parts();
        let data = C::decode(&raw).map_err(HttpTypedError::Codec)?;
        let resp = Response::from_parts(parts, data);

        Ok(resp)
    }
}

pub trait HttpExt: HttpClient {
    fn get(&self, url: &str) -> impl Future<Output = Result<Bytes, HttpExtError<Self::Error>>>;
}

impl<HC: HttpClient> HttpExt for HC {
    async fn get(&self, url: &str) -> Result<Bytes, HttpExtError<Self::Error>> {
        let request = Request::builder()
            .method(Method::GET)
            .uri(url)
            .body(Bytes::new())
            .map_err(HttpExtError::Request)?;

        let resp = self.send(request).await?;

        Ok(resp.into_body())
    }
}

pub trait HttpExtTyped<C: Codec>: HttpExt {
    fn get_typed<V: DeserializeOwned>(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<V, HttpTypedError<C::Error, HttpExtError<Self::Error>>>>;
}

impl<HE: HttpExt, C: Codec> HttpExtTyped<C> for HE {
    async fn get_typed<V: DeserializeOwned>(
        &self,
        url: &str,
    ) -> Result<V, HttpTypedError<C::Error, HttpExtError<Self::Error>>> {
        let raw = self.get(url).await?;

        let data = C::decode(&raw).map_err(HttpTypedError::Codec)?;

        Ok(data)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HttpExtError<ClientErr: std::error::Error> {
    #[error("request: {0}")]
    Request(http::Error),

    #[error(transparent)]
    Client(#[from] ClientErr),
}

#[derive(Debug, thiserror::Error)]
pub enum HttpTypedError<CodecErr: std::error::Error, ClientErr: std::error::Error> {
    #[error("codec: {0}")]
    Codec(CodecErr),

    #[error(transparent)]
    Client(#[from] ClientErr),
}
