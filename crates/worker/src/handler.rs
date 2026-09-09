use std::pin::Pin;

use alternate_codec::Codec;
use anyhow::Context as _;
use serde::de::DeserializeOwned;
use tracing::Instrument as _;

#[derive(Clone)]
pub struct HandlerData<S> {
    pub data: Vec<u8>,
    pub state: S,
}

#[allow(clippy::type_complexity)]
pub fn state_handler<
    S: Clone + Send + 'static,
    Fut: Future<Output = Result<(), HandlerError>> + Send,
    F: Fn(S) -> Fut + Clone + Send + 'static,
>(
    handler: F,
) -> impl Fn(HandlerData<S>) -> Pin<Box<dyn Future<Output = Result<(), HandlerError>> + Send>> {
    move |req: HandlerData<S>| {
        let handler = handler.clone();

        Box::pin(async move {
            let span = tracing::info_span!("worker", worker_type = %std::any::type_name::<F>());

            async move { handler(req.state).await }
                .instrument(span)
                .await
        })
    }
}

#[allow(clippy::type_complexity)]
pub fn data_handler<
    C: Codec,
    D: DeserializeOwned,
    S: Clone + Send + 'static,
    Fut: Future<Output = Result<(), HandlerError>> + Send,
    F: Fn(D, S) -> Fut + Clone + Send + 'static,
>(
    handler: F,
) -> impl Fn(HandlerData<S>) -> Pin<Box<dyn Future<Output = Result<(), HandlerError>> + Send>>
where
    C::Error: Send + Sync + 'static,
{
    move |req: HandlerData<S>| {
        let handler = handler.clone();

        Box::pin(async move {
            let span = tracing::info_span!("worker", worker_type = %std::any::type_name::<F>(), worker_data = %std::any::type_name::<D>());

            async move {
                let data = C::decode::<D>(&req.data).context("decode handler data")?;

                handler(data, req.state).await
            }
            .instrument(span)
            .await
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct HandlerError(#[from] anyhow::Error);
