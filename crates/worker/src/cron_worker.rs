use chrono::Utc;
use cron::Schedule;
use tower::{Service as _, ServiceExt as _, util::BoxService};

use crate::{HandlerError, handler::HandlerData};

pub struct CronWorker<S> {
    schedule: Schedule,
    handler: BoxService<HandlerData<S>, (), HandlerError>,
    state: S,
}

impl<S: Clone> CronWorker<S> {
    pub fn new(
        schedule: Schedule,
        handler: BoxService<HandlerData<S>, (), HandlerError>,
        state: S,
    ) -> Self {
        Self {
            schedule,
            handler,
            state,
        }
    }

    /// # Panics
    ///
    /// Will panic if the cron has no next scheduled time
    pub async fn start(mut self) {
        loop {
            let upcoming = self.schedule.upcoming(Utc).take(1).next().unwrap();

            let duration = (upcoming - Utc::now()).to_std().unwrap();

            tokio::time::sleep(duration).await;

            if let Err(e) = self
                .handler
                .ready()
                .await
                .unwrap()
                .call(HandlerData {
                    data: Vec::new(),
                    state: self.state.clone(),
                })
                .await
            {
                tracing::error!(
                    error = %e,
                    error_debug = ?e,
                    "worker handler failed"
                );
            }
        }
    }
}
