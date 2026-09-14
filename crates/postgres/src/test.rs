use std::{
    ops::Deref,
    sync::{Arc, Weak},
};

use alternate_migration::{AsyncMigrationRunner, Migration, postgres::PostgresBackend};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{IntoContainerPort as _, WaitFor, wait::LogWaitStrategy},
    runners::AsyncRunner as _,
};
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls};

const PORT: u16 = 5432;
const PASSWORD: &str = "postgres";

static POSTGRES_SERVER: Mutex<Weak<PostgresServer>> = Mutex::const_new(Weak::new());

pub struct PostgresServer {
    _container: ContainerAsync<GenericImage>,
    connection_string: String,
}

impl PostgresServer {
    pub async fn init(
        migrations: Vec<Migration>,
    ) -> Result<Arc<PostgresServer>, Box<dyn std::error::Error>> {
        let mut cached = POSTGRES_SERVER.lock().await;

        if let Some(db) = cached.upgrade() {
            return Ok(db);
        }

        let container = GenericImage::new("postgres", "18-alpine")
            .with_exposed_port(PORT.tcp())
            .with_wait_for(WaitFor::Log(
                LogWaitStrategy::stdout_or_stderr("database system is ready to accept connections")
                    .with_times(2),
            ))
            .with_env_var("POSTGRES_PASSWORD", PASSWORD)
            .start()
            .await?;

        let host = container.get_host().await?.to_string();
        let port = container.get_host_port_ipv4(PORT).await?;

        let connection_string =
            format!("host={host} port={port} user=postgres password={PASSWORD} dbname=postgres");

        {
            let (client, connection) = tokio_postgres::connect(&connection_string, NoTls).await?;
            tokio::spawn(connection);

            let mut migration_runner = AsyncMigrationRunner::new(PostgresBackend::new(client));
            migration_runner.migrate_to_latest(migrations).await?;
        }

        let db = Arc::new(PostgresServer {
            _container: container,
            connection_string,
        });
        *cached = Arc::downgrade(&db);

        Ok(db)
    }

    pub fn connection_string(&self) -> &str {
        &self.connection_string
    }
}

pub struct PostgresClient {
    client: Client,
    _db: Arc<PostgresServer>,
}

impl Deref for PostgresClient {
    type Target = Client;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

pub async fn postgres_client(
    migrations: Vec<Migration>,
) -> Result<PostgresClient, Box<dyn std::error::Error>> {
    let db = PostgresServer::init(migrations).await?;

    let (client, connection) = tokio_postgres::connect(&db.connection_string, NoTls).await?;
    tokio::spawn(connection);

    Ok(PostgresClient { client, _db: db })
}
