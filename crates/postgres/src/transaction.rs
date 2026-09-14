use alternate_domain::{TransactionScope, TransactionScopeFactory};
use deadpool_postgres::{Client, Object, Pool, PoolError};
use tokio_postgres::Error as PostgresError;

#[derive(Debug, Clone)]
pub struct PostgresTransactionScopeFactory {
    pool: Pool,
}

impl PostgresTransactionScopeFactory {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

impl TransactionScopeFactory for PostgresTransactionScopeFactory {
    type Scope = PostgresTransactionScope;

    async fn begin(&self) -> Result<Self::Scope, PostgresTransactionError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(PostgresTransactionError::Pool)?;

        client
            .batch_execute("BEGIN")
            .await
            .map_err(PostgresTransactionError::Begin)?;

        Ok(PostgresTransactionScope {
            client: Some(client),
        })
    }
}

#[derive(Debug)]
pub struct PostgresTransactionScope {
    client: Option<Client>,
}

impl PostgresTransactionScope {
    /// # Panics
    ///
    /// Will panic if transaction already finalized
    pub fn client(&self) -> &Object {
        self.client.as_ref().expect("already finalized")
    }
}

impl TransactionScope for PostgresTransactionScope {
    type Error = PostgresTransactionError;

    async fn commit(mut self) -> Result<(), Self::Error> {
        let client = self.client.take().expect("already finalized");

        client
            .batch_execute("COMMIT")
            .await
            .map_err(PostgresTransactionError::Commit)
    }

    async fn rollback(mut self) -> Result<(), Self::Error> {
        let client = self.client.take().expect("already finalized");

        client
            .batch_execute("ROLLBACK")
            .await
            .map_err(PostgresTransactionError::Rollback)
    }
}

impl Drop for PostgresTransactionScope {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            tokio::spawn(async move {
                let _ = client.batch_execute("ROLLBACK").await;
            });
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PostgresTransactionError {
    #[error("begin: {0}")]
    Begin(PostgresError),

    #[error("commit: {0}")]
    Commit(PostgresError),

    #[error("rollback: {0}")]
    Rollback(PostgresError),

    #[error("rollback: {0}")]
    Pool(PoolError),
}
