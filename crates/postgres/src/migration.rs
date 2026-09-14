use alternate_migration::{Migration, MigrationError};

#[derive(rust_embed::Embed)]
#[folder = "migrations/postgres"]
struct Migrations;

/// # Panics
///
/// Will panic if a migration file is not found
pub fn migrations() -> Result<Vec<Migration>, MigrationError> {
    Migrations::iter()
        .map(|filename| {
            let migration = Migrations::get(&filename).expect("migration file not found");
            let sql = String::from_utf8_lossy(migration.data.as_ref()).to_string();

            Migration::try_new(&filename, sql)
        })
        .collect()
}
