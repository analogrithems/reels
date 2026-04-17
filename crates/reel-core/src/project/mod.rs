//! Serializable project model and the in-memory store that owns it.
//!
//! There are no third-party consumers of `project.json` yet — the schema is
//! allowed to evolve. Prefer explicit fields + [`schema::migrate`] over
//! open-ended compatibility workarounds until a stable release is declared.

mod autosave;
mod orientation;
mod schema;

pub use autosave::ProjectStore;
pub use orientation::ClipOrientation;
pub use schema::{migrate, MigrationError, SCHEMA_VERSION};

use serde::{Deserialize, Serialize};
use serde_json::Map;
use std::path::PathBuf;
use uuid::Uuid;

use crate::media::MediaMetadata;

/// Kinds of tracks supported by the Phase 0–2 model.
///
/// Richer kinds (subtitle, effect) come later; the enum is non-exhaustive so
/// new variants can be added without breaking serde of older JSON (unknown
/// values can be rejected until a schema bump adds the variant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum TrackKind {
    Video,
    Audio,
}

/// A single clip referenced from a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Uuid,
    pub source_path: PathBuf,
    pub metadata: MediaMetadata,
    /// In-point, seconds from the start of the source media.
    pub in_point: f64,
    /// Out-point, seconds from the start of the source media.
    pub out_point: f64,
    /// User-applied rotate/flip (QuickTime-style). Defaults to identity and is
    /// omitted from JSON in that case so existing snapshots stay stable.
    #[serde(default, skip_serializing_if = "ClipOrientation::is_identity")]
    pub orientation: ClipOrientation,
    /// Future filter graphs, AI params, etc. Unknown keys round-trip here.
    #[serde(flatten)]
    pub extensions: Map<String, serde_json::Value>,
}

/// An ordered list of clip ids that render into one logical channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: Uuid,
    pub kind: TrackKind,
    pub clip_ids: Vec<Uuid>,
    #[serde(flatten)]
    pub extensions: Map<String, serde_json::Value>,
}

/// The root serializable project object. Persisted as `project.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Current schema version; bump when the on-disk format changes.
    pub schema_version: u32,
    pub name: String,

    /// Disk location; skipped from serde because it *is* the location.
    #[serde(skip)]
    pub path: Option<PathBuf>,

    pub clips: Vec<Clip>,
    pub tracks: Vec<Track>,

    /// ISO-8601 UTC; stored as plain String so we don't drag in chrono.
    pub created_at: String,
    pub modified_at: String,
    /// App-wide metadata (e.g. default AI settings). Unknown top-level JSON keys
    /// round-trip here so newer writers can add fields older builds ignore.
    #[serde(flatten)]
    pub extensions: Map<String, serde_json::Value>,
}

impl Project {
    /// Construct a new empty project with the given name and `created_at ==
    /// modified_at == now`.
    pub fn new(name: impl Into<String>) -> Self {
        let now = now_iso8601();
        Project {
            schema_version: SCHEMA_VERSION,
            name: name.into(),
            path: None,
            clips: Vec::new(),
            tracks: Vec::new(),
            created_at: now.clone(),
            modified_at: now,
            extensions: Map::new(),
        }
    }

    /// Update `modified_at` to now.
    pub fn touch(&mut self) {
        self.modified_at = now_iso8601();
    }
}

/// ISO-8601 in UTC, second precision, `Z` suffix.
///
/// Computed from `SystemTime::UNIX_EPOCH`; zero deps, good enough for a
/// file-timestamp field that's never parsed back. (If we ever need parsing,
/// swap to `jiff`.)
pub(crate) fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_iso8601_from_unix(secs)
}

/// Pure function separated out for testability.
pub(crate) fn format_iso8601_from_unix(secs: u64) -> String {
    // Days since 1970-01-01.
    let days = (secs / 86_400) as i64;
    let time_of_day = secs % 86_400;
    let hour = (time_of_day / 3600) as u32;
    let minute = ((time_of_day % 3600) / 60) as u32;
    let second = (time_of_day % 60) as u32;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_formats_epoch() {
        assert_eq!(format_iso8601_from_unix(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn iso8601_formats_known_stamp() {
        // 2021-01-01T00:00:00Z = 1 609 459 200 seconds since epoch (well-known).
        assert_eq!(
            format_iso8601_from_unix(1_609_459_200),
            "2021-01-01T00:00:00Z"
        );
        // 1 000 000 000 epoch = 2001-09-09T01:46:40Z (Wikipedia-verified).
        assert_eq!(
            format_iso8601_from_unix(1_000_000_000),
            "2001-09-09T01:46:40Z"
        );
        // 1 234 567 890 epoch = 2009-02-13T23:31:30Z (Wikipedia-verified).
        assert_eq!(
            format_iso8601_from_unix(1_234_567_890),
            "2009-02-13T23:31:30Z"
        );
    }

    #[test]
    fn new_project_has_matching_timestamps() {
        let p = Project::new("test");
        assert_eq!(p.created_at, p.modified_at);
        assert_eq!(p.schema_version, SCHEMA_VERSION);
    }
}
