//! Project schema version and forward-migration hook.
//!
//! Reel is still pre-release and nothing outside this repo depends on
//! `project.json` yet. When the on-disk shape needs to change, bump
//! [`SCHEMA_VERSION`], extend [`migrate`], and update tests — favor a clean
//! model over carrying legacy baggage.

/// On-disk schema version for `Project`. Bump whenever a breaking field
/// rename/removal lands; add migrations in [`migrate`].
pub const SCHEMA_VERSION: u32 = 2;

/// Migrate a parsed JSON value to the latest schema if possible.
///
/// Rewrites legacy `schema_version` values in-place before
/// [`serde_json::from_value`] builds a [`super::Project`]. While Reel is
/// pre-release, keep this minimal; older versions exist mainly for tests and
/// early experiments.
pub fn migrate(value: &mut serde_json::Value) -> Result<(), MigrationError> {
    let detected = value
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .ok_or(MigrationError::MissingSchemaVersion)? as u32;

    match detected {
        1 => {
            value["schema_version"] = serde_json::json!(SCHEMA_VERSION);
            Ok(())
        }
        SCHEMA_VERSION => Ok(()),
        other => Err(MigrationError::Unsupported {
            detected: other,
            latest: SCHEMA_VERSION,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_bumps_v1_to_current() {
        let mut v = serde_json::json!({
            "schema_version": 1,
            "name": "legacy",
            "clips": [],
            "tracks": [],
            "created_at": "2026-01-01T00:00:00Z",
            "modified_at": "2026-01-01T00:00:00Z"
        });
        migrate(&mut v).unwrap();
        assert_eq!(v["schema_version"], SCHEMA_VERSION);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("`schema_version` is missing from the project JSON")]
    MissingSchemaVersion,

    #[error("unsupported schema version {detected} (this build supports v{latest})")]
    Unsupported { detected: u32, latest: u32 },
}
