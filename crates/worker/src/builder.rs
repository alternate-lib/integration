use std::future::Future;

use alternate_queue::{QueueConsumer, QueueLeaseReclaim, QueueRejection, QueueScheduledPromotion};
use tower::{ServiceBuilder, ServiceExt as _, util::BoxService};

use crate::{
    handler::{HandlerData, HandlerError},
    queue_worker::{QueueWorker, RetryPolicy},
};

pub struct WorkerBuilder<B, S> {
    backend: B,
    service: BoxService<HandlerData<S>, (), HandlerError>,
}

impl<B, S> WorkerBuilder<B, S> {
    pub fn new<F, Fut>(backend: B, handler: F) -> Self
    where
        F: Fn(HandlerData<S>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), HandlerError>> + Send + 'static,
        HandlerData<S>: Send + 'static,
        S: Clone + Send + Sync + 'static,
    {
        let service = ServiceBuilder::new().service_fn(handler).boxed();

        Self { backend, service }
    }

    #[must_use]
    pub fn concurrency(self, n: usize) -> Self
    where
        HandlerData<S>: Send + 'static,
    {
        let WorkerBuilder { backend, service } = self;

        let service = ServiceBuilder::new()
            .concurrency_limit(n)
            .service(service)
            .boxed();

        Self { backend, service }
    }

    pub fn build(self, state: S, retry_policy: RetryPolicy) -> QueueWorker<B, S>
    where
        B: QueueConsumer + QueueRejection + QueueLeaseReclaim + QueueScheduledPromotion,
        S: Clone,
    {
        QueueWorker::new(self.backend, self.service, state, retry_policy)
    }
}
