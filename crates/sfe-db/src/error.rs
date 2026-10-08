#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("database failed integrity check: {0}")]
    Corrupt(String),
    #[error("row has invalid value in column `{column}`: {value}")]
    InvalidColumn { column: &'static str, value: String },
}

impl DbError {
    pub(crate) fn invalid(column: &'static str, value: impl ToString) -> Self {
        DbError::InvalidColumn {
            column,
            value: value.to_string(),
        }
    }
}
