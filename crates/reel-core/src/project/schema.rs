//! Project schema version and forward-migration hook.

/// On-disk schema version for `Project`. Bump whenever a breaking field
/// rename/removal lands; add migrations in [`migrate`].
pub const SCHEMA_VERSION: u32 = 1;

/// Migrate a parsed JSON value to the latest schema if possible.
///
/// Phase 0–2 is a no-op: we only know version 1. Future versions will rewrite
/// legacy shapes in-place before serde_from_value is called.
pub fn migrate(value: &mut serde_json::Value) -> Result<(), MigrationError> {
    let detected = value
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .ok_or(MigrationError::MissingSchemaVersion)? as u32;

    match detected {
        SCHEMA_VERSION => Ok(()),
        other => Err(MigrationError::Unsupported {
            detected: other,
            latest: SCHEMA_VERSION,
        }),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("`schema_version` is missing from the project JSON")]
    MissingSchemaVersion,

    #[error("unsupported schema version {detected} (this build supports v{latest})")]
    Unsupported { detected: u32, latest: u32 },
}
