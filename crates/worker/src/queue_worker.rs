use std::time::Duration;

use alternate_queue::{
    QueueConsumer, QueueLeaseReclaim, QueueRejection, QueueScheduledPromotion, RejectAction,
};
use tower::{Service as _, ServiceExt as _, util::BoxService};

use crate::handler::{HandlerData, HandlerError};

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub backoff: Backoff,
}

impl RetryPolicy {
    pub fn new(backoff: Backoff) -> Self {
        Self { backoff }
    }
}

#[derive(Clone, Debug)]
pub enum Backoff {
    None,
    Fixed(Duration),
    Exponential {
        base: Duration,
        factor: f64,
        cap: Duration,
    },
}

impl Backoff {
    /// # Panics
    ///
    /// Will panic if attempt count cannot fit into i32
    pub fn retry_after(&self, attempt: usize) -> Option<Duration> {
        match self {
            Backoff::None => None,
            Backoff::Fixed(d) => Some(*d),
            Backoff::Exponential { base, factor, cap } => {
                let secs = base.as_secs_f64() * factor.powi(i32::try_from(attempt).unwrap());
                Some(Duration::from_secs_f64(secs.min(cap.as_secs_f64())))
            }
        }
    }
}

pub struct QueueWorker<JC, S> {
    backend: JC,
    handler: BoxService<HandlerData<S>, (), HandlerError>,
    state: S,
    retry_policy: RetryPolicy,
    reclaim_interval: Duration,
    promote_interval: Duration,
}

impl<JC, S: Clone> QueueWorker<JC, S> {
    pub fn new(
        backend: JC,
        handler: BoxService<HandlerData<S>, (), HandlerError>,
        state: S,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            backend,
            handler,
            state,
            retry_policy,
            reclaim_interval: Duration::from_secs(30),
            promote_interval: Duration::from_secs(10),
        }
    }

    #[must_use]
    pub fn with_reclaim_interval(mut self, interval: Duration) -> Self {
        self.reclaim_interval = interval;
        self
    }

    #[must_use]
    pub fn with_promote_interval(mut self, interval: Duration) -> Self {
        self.promote_interval = interval;
        self
    }
}

impl<JC, S> QueueWorker<JC, S>
where
    JC: QueueConsumer + QueueRejection + QueueLeaseReclaim + QueueScheduledPromotion,
    S: Clone,
{
    /// # Panics
    ///
    /// Will panic if the handler service fails to become ready
    pub async fn start(mut self) {
        let mut reclaim_tick = tokio::time::interval(self.reclaim_interval);
        let mut promote_tick = tokio::time::interval(self.promote_interval);

        loop {
            let delivery = tokio::select! {
                biased;

                _ = reclaim_tick.tick() => {
                    self.reclaim().await;
                    continue;
                }

                _ = promote_tick.tick() => {
                    self.promote().await;
                    continue;
                }

                res = self.backend.receive() => match res {
                    Ok(Some(delivery)) => delivery,
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::error!(error = %e, error_debug = ?e, "failed to dequeue message");
                        continue;
                    }
                },
            };

            let attempt = delivery.attempts;

            match self
                .handler
                .ready()
                .await
                .unwrap()
                .call(HandlerData {
                    data: delivery.payload,
                    state: self.state.clone(),
                })
                .await
            {
                Ok(()) => {
                    if let Err(e) = self.backend.ack(delivery.receipt).await {
                        tracing::error!(error = %e, error_debug = ?e, "failed to ack message");
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, error_debug = ?e, "worker handler failed");

                    let retry_after = self.retry_policy.backoff.retry_after(attempt);

                    match self
                        .backend
                        .reject(
                            delivery.receipt,
                            RejectAction::Retry {
                                after: retry_after,
                                max_attempts: None,
                            },
                        )
                        .await
                    {
                        Ok(outcome) => {
                            if outcome.exhausted {
                                tracing::warn!(
                                    attempts = outcome.attempts,
                                    "message dead-lettered (retry budget exhausted)"
                                );
                            } else {
                                tracing::warn!(
                                    attempts = outcome.attempts,
                                    retry_after = ?retry_after,
                                    "message requeued for retry"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                error = %e,
                                error_debug = ?e,
                                "failed to reject message"
                            );
                        }
                    }
                }
            }
        }
    }

    async fn reclaim(&self) {
        match self.backend.reclaim().await {
            Ok(moved) => {
                if moved > 0 {
                    tracing::debug!(reclaimed = moved, "reclaimed orphaned messages");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, error_debug = ?e, "reclaim_orphaned failed");
            }
        }
    }

    async fn promote(&self) {
        match self.backend.promote().await {
            Ok(moved) => {
                if moved > 0 {
                    tracing::debug!(promoted = moved, "promoted due messages");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, error_debug = ?e, "promote_due failed");
            }
        }
    }
}
