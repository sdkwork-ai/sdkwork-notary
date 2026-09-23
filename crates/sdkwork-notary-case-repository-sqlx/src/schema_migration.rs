//! Canonical notary schema SQL sourced from `database/ddl/baseline/`.

/// Returns the postgres foundation baseline for host bootstrap helpers.
pub fn notary_foundation_postgres_migration_sql() -> &'static str {
    include_str!("../../../database/ddl/baseline/postgres/0001_notary_baseline.sql")
}
