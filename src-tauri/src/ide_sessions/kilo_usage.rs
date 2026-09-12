//! Token watermarking for Kilo's SQLite event stream.
//!
//! The database observer supplies only `json_extract`ed metadata.  In particular,
//! this module never receives a message body or a part payload: its input is the
//! assistant message id plus the token snapshot Kilo stores on that message.

use std::collections::HashMap;

use crate::types::{SessionEventTokens, SessionModelTokens};

/// Metadata selected from a `message.updated.1` row.  The SQLite reader must use
/// JSON paths for these values and must not select the message content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageRow {
    pub id: String,
    pub role: Option<String>,
    pub model: Option<String>,
    /// Kilo reports input excluding cache read/write already.
    pub input: Option<u64>,
    pub output: Option<u64>,
    /// Kilo records reasoning separately from visible output.
    pub reasoning: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    model: Option<String>,
}

/// Tracks Kilo's cumulative per-assistant-message token snapshots.
///
/// Attachment rows are seeded through [`Self::with_baseline`], so an observer
/// never charges a reply that predates its connection.  Later rows emit only the
/// increase for their message id.  A reduced/replayed snapshot leaves the last
/// watermark intact, preventing an eventual replay from being charged twice.
#[derive(Debug, Default)]
pub struct KiloUsageTracker {
    snapshots: HashMap<String, Snapshot>,
}

impl KiloUsageTracker {
    pub fn with_baseline(rows: impl IntoIterator<Item = UsageRow>) -> Self {
        let mut tracker = Self::default();
        for row in rows {
            tracker.seed(row);
        }
        tracker
    }

    /// Adds a single metadata-only event and returns its unbilled token increase.
    /// The caller owns turn boundaries and can accumulate this result until Stop.
    pub fn observe(&mut self, row: &UsageRow) -> Option<SessionEventTokens> {
        if !is_assistant(row) {
            return None;
        }
        let next = Snapshot::from(row);
        let previous = match self.snapshots.get(&row.id).cloned() {
            Some(previous) => previous,
            // A new id appeared after attachment, so its first snapshot is a new
            // assistant message rather than historic usage.
            None => {
                self.snapshots.insert(row.id.clone(), next.clone());
                return delta(&next, &Snapshot::default());
            }
        };

        // Counters are cumulative snapshots.  Do not advance the watermark on a
        // stale/replayed or reset record: the next valid increase must compare to
        // the largest previously accepted snapshot.
        if next.input < previous.input
            || next.output < previous.output
            || next.cache_read < previous.cache_read
            || next.cache_write < previous.cache_write
        {
            return None;
        }
        self.snapshots.insert(row.id.clone(), next.clone());
        delta(&next, &previous)
    }

    pub fn seed(&mut self, row: UsageRow) {
        if is_assistant(&row) {
            let next = Snapshot::from(&row);
            // A baseline scan can contain replayed entries.  Keep the largest
            // complete snapshot for each id, without producing an event.
            match self.snapshots.get(&row.id) {
                Some(previous)
                    if next.input < previous.input
                        || next.output < previous.output
                        || next.cache_read < previous.cache_read
                        || next.cache_write < previous.cache_write => {}
                _ => {
                    self.snapshots.insert(row.id, next);
                }
            }
        }
    }
}

impl From<&UsageRow> for Snapshot {
    fn from(row: &UsageRow) -> Self {
        Self {
            input: row.input.unwrap_or(0),
            output: row
                .output
                .unwrap_or(0)
                .saturating_add(row.reasoning.unwrap_or(0)),
            cache_read: row.cache_read.unwrap_or(0),
            cache_write: row.cache_write.unwrap_or(0),
            model: row
                .model
                .as_deref()
                .filter(|model| !model.trim().is_empty())
                .map(str::to_owned),
        }
    }
}

fn is_assistant(row: &UsageRow) -> bool {
    !row.id.is_empty() && row.role.as_deref() == Some("assistant")
}

fn delta(next: &Snapshot, previous: &Snapshot) -> Option<SessionEventTokens> {
    let input = next.input - previous.input;
    let output = next.output - previous.output;
    let cache_read = next.cache_read - previous.cache_read;
    let cache_write = next.cache_write - previous.cache_write;
    let model = next.model.clone().or_else(|| previous.model.clone());
    SessionEventTokens {
        input: Some(input),
        output: Some(output),
        cache_read: Some(cache_read),
        cache_write: Some(cache_write),
        model: model.clone(),
        by_model: model.map(|model| {
            vec![SessionModelTokens {
                input: Some(input),
                output: Some(output),
                cache_read: Some(cache_read),
                cache_write: Some(cache_write),
                model: Some(model),
            }]
        }),
    }
    .non_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, input: u64, output: u64, model: &str) -> UsageRow {
        UsageRow {
            id: id.into(),
            role: Some("assistant".into()),
            model: Some(model.into()),
            input: Some(input),
            output: Some(output),
            reasoning: None,
            cache_read: Some(0),
            cache_write: Some(0),
        }
    }

    #[test]
    fn baseline_and_replays_do_not_charge_history() {
        let mut tracker = KiloUsageTracker::with_baseline([row("old", 100, 10, "m")]);
        assert_eq!(tracker.observe(&row("old", 100, 10, "m")), None);
        assert_eq!(tracker.observe(&row("old", 90, 9, "m")), None);
        let delta = tracker.observe(&row("old", 105, 12, "m")).unwrap();
        assert_eq!((delta.input, delta.output), (Some(5), Some(2)));
    }

    #[test]
    fn new_assistant_snapshot_counts_output_and_reasoning_once() {
        let mut tracker = KiloUsageTracker::default();
        let mut first = row("new", 20, 3, "kilo/model-a");
        first.reasoning = Some(7);
        first.cache_read = Some(2);
        let tokens = tracker.observe(&first).unwrap();
        assert_eq!(tokens.input, Some(20));
        assert_eq!(tokens.output, Some(10));
        assert_eq!(tokens.cache_read, Some(2));
        assert_eq!(
            tokens.by_model.unwrap()[0].model.as_deref(),
            Some("kilo/model-a")
        );
        assert_eq!(tracker.observe(&first), None);
    }

    #[test]
    fn model_change_attributes_the_new_increment_to_the_new_model() {
        let mut tracker = KiloUsageTracker::default();
        assert!(tracker.observe(&row("a", 10, 1, "model-a")).is_some());
        let delta = tracker.observe(&row("a", 13, 5, "model-b")).unwrap();
        assert_eq!(delta.model.as_deref(), Some("model-b"));
        assert_eq!((delta.input, delta.output), (Some(3), Some(4)));
        assert_eq!(delta.by_model.unwrap()[0].model.as_deref(), Some("model-b"));
    }

    #[test]
    fn non_assistant_rows_and_counter_decreases_are_ignored() {
        let mut tracker = KiloUsageTracker::default();
        let mut user = row("u", 100, 10, "m");
        user.role = Some("user".into());
        assert_eq!(tracker.observe(&user), None);
        assert!(tracker.observe(&row("a", 10, 10, "m")).is_some());
        assert_eq!(tracker.observe(&row("a", 11, 9, "m")), None);
        let delta = tracker.observe(&row("a", 12, 12, "m")).unwrap();
        assert_eq!((delta.input, delta.output), (Some(2), Some(2)));
    }
}
