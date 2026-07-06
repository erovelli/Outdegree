//! Pure helpers for the data-stewardship surface (§8): the backup-nudge decision
//! and human-readable byte formatting.
//!
//! Kept in the pure (native-testable) layer — like [`crate::ui_prefs`] and
//! [`crate::views`] — so the decision table (counts/timestamps → show/snooze/hide)
//! and the byte formatting are unit-tested under `cargo test`. The wasm shell in
//! `ui`/`store` only supplies the live inputs (event counts, `meta` timestamps,
//! `navigator.storage.estimate()`), never the logic.

/// Only nudge once the log is large enough that losing it would actually hurt —
/// below this a backup is cheap to recreate by browsing, so staying silent avoids
/// nagging new users. Strictly greater-than: the nudge starts *above* this count.
pub const NUDGE_EVENT_THRESHOLD: u64 = 5_000;

/// A backup is considered stale once it is older than 60 days.
pub const BACKUP_STALE_MS: f64 = 60.0 * 86_400_000.0;

/// "Snooze" hides the nudge for 30 days.
pub const SNOOZE_MS: f64 = 30.0 * 86_400_000.0;

/// What the dashboard should do with the backup nudge, given the current data
/// volume and the persisted backup/snooze timestamps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nudge {
    /// Surface the "time to back up" chip.
    Show,
    /// Eligible by data volume and staleness, but the user snoozed it — stay
    /// hidden until the snooze elapses. Kept distinct from [`Nudge::Hidden`] so
    /// the decision is fully testable (and self-documenting).
    Snoozed,
    /// Nothing to nudge: too little data, or a recent-enough backup.
    Hidden,
}

/// Decide whether to show the backup nudge (§8.4).
///
/// Shows only when **all** hold: the event log exceeds
/// [`NUDGE_EVENT_THRESHOLD`]; there is no backup within [`BACKUP_STALE_MS`]
/// (`last_export_ts` absent, or older than that); and any snooze
/// (`snoozed_until`) has elapsed (`now` is past it). The intermediate
/// [`Nudge::Snoozed`] vs. [`Nudge::Hidden`] distinction is informational — both
/// keep the chip hidden — but makes the reason explicit and testable.
///
/// All timestamps are epoch milliseconds (as from `Date.now()`); `None` means the
/// corresponding `meta` key was never written.
pub fn nudge_state(
    events: u64,
    last_export_ts: Option<f64>,
    snoozed_until: Option<f64>,
    now: f64,
) -> Nudge {
    // Not enough data to be worth a backup reminder yet.
    if events <= NUDGE_EVENT_THRESHOLD {
        return Nudge::Hidden;
    }
    // A recent-enough backup exists → nothing to nudge.
    if let Some(ts) = last_export_ts {
        if now - ts < BACKUP_STALE_MS {
            return Nudge::Hidden;
        }
    }
    // Eligible, but currently snoozed.
    if let Some(until) = snoozed_until {
        if now <= until {
            return Nudge::Snoozed;
        }
    }
    Nudge::Show
}

/// Format a byte count as a compact human-readable string (`"512 B"`,
/// `"1.0 KB"`, `"48.2 MB"`), binary (1024) scaled. Used for the settings
/// "Storage" readout; the byte figure comes from `navigator.storage.estimate()`.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: f64 = 86_400_000.0;
    const NOW: f64 = 1_700_000_000_000.0;

    #[test]
    fn hidden_below_the_event_threshold() {
        // Even with no backup ever, too little data stays silent (never nags a
        // fresh install). The threshold is exclusive.
        assert_eq!(nudge_state(0, None, None, NOW), Nudge::Hidden);
        assert_eq!(
            nudge_state(NUDGE_EVENT_THRESHOLD, None, None, NOW),
            Nudge::Hidden
        );
    }

    #[test]
    fn shows_when_over_threshold_and_never_backed_up() {
        assert_eq!(
            nudge_state(NUDGE_EVENT_THRESHOLD + 1, None, None, NOW),
            Nudge::Show
        );
        assert_eq!(nudge_state(50_000, None, None, NOW), Nudge::Show);
    }

    #[test]
    fn hidden_after_a_recent_backup() {
        // A backup 10 days ago is fresh; 60 days ago is exactly stale (shows).
        assert_eq!(
            nudge_state(50_000, Some(NOW - 10.0 * DAY), None, NOW),
            Nudge::Hidden
        );
        assert_eq!(
            nudge_state(50_000, Some(NOW - 59.0 * DAY), None, NOW),
            Nudge::Hidden
        );
        assert_eq!(
            nudge_state(50_000, Some(NOW - 61.0 * DAY), None, NOW),
            Nudge::Show
        );
    }

    #[test]
    fn snoozed_hides_until_it_elapses() {
        // Eligible (lots of data, no recent backup) but snoozed into the future.
        assert_eq!(
            nudge_state(50_000, None, Some(NOW + 5.0 * DAY), NOW),
            Nudge::Snoozed
        );
        // Snooze in the past → eligible again.
        assert_eq!(
            nudge_state(50_000, None, Some(NOW - 1.0 * DAY), NOW),
            Nudge::Show
        );
    }

    #[test]
    fn recent_backup_wins_over_an_active_snooze() {
        // A fresh backup means there is simply nothing to nudge, regardless of a
        // lingering snooze value — reported as Hidden, not Snoozed.
        assert_eq!(
            nudge_state(50_000, Some(NOW - 1.0 * DAY), Some(NOW + 5.0 * DAY), NOW),
            Nudge::Hidden
        );
    }

    #[test]
    fn human_bytes_scales_and_formats() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(50_534_154), "48.2 MB");
        assert_eq!(human_bytes(5 * 1024 * 1024 * 1024), "5.0 GB");
    }
}
