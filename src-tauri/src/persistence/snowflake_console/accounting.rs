//! Checked logical charges for the existing journal representation. These are
//! neither filesystem reservations nor authority to admit or settle a run.

use super::{MAX_JSON_BYTES, MAX_STEPS, invalid};
use crate::domain::AppResult;

pub(super) const RUN_OVERHEAD_BYTES: u64 = 512;
pub(super) const STEP_OVERHEAD_BYTES: u64 = 256;
pub(super) const REFUSAL_OVERHEAD_BYTES: u64 = 256;

// Match the bounded fields read by read_run/read_rejection, including persisted
// UTF-8 JSON bytes rather than decoded strings or Unicode character counts.
const MAX_RUN_PAYLOAD_BYTES: u64 = 4 * 36 + 64 + 8 + 2 * MAX_JSON_BYTES as u64;
const MAX_STEP_PAYLOAD_BYTES: u64 = 2 * MAX_JSON_BYTES as u64;
const MAX_REFUSAL_PAYLOAD_BYTES: u64 = 2 * 36 + 64 + 64;
const MAX_POSITIVE_CHARGE_BYTES: u64 = RUN_OVERHEAD_BYTES
    + MAX_RUN_PAYLOAD_BYTES
    + MAX_STEPS as u64 * (STEP_OVERHEAD_BYTES + MAX_STEP_PAYLOAD_BYTES);
const MAX_REFUSAL_CHARGE_BYTES: u64 = REFUSAL_OVERHEAD_BYTES + MAX_REFUSAL_PAYLOAD_BYTES;

#[derive(Clone, Copy, Debug)]
pub(super) enum RecordKind {
    Run,
    Step,
    Refusal,
}

fn sqlite_total(value: u64) -> AppResult<i64> {
    i64::try_from(value).map_err(|_| invalid())
}

pub(super) fn checked_charge(kind: RecordKind, payload_bytes: u64) -> AppResult<u64> {
    let (overhead, maximum) = match kind {
        RecordKind::Run => (RUN_OVERHEAD_BYTES, MAX_RUN_PAYLOAD_BYTES),
        RecordKind::Step => (STEP_OVERHEAD_BYTES, MAX_STEP_PAYLOAD_BYTES),
        RecordKind::Refusal => (REFUSAL_OVERHEAD_BYTES, MAX_REFUSAL_PAYLOAD_BYTES),
    };
    let charge = overhead.checked_add(payload_bytes).ok_or_else(invalid)?;
    sqlite_total(charge)?;
    if payload_bytes > maximum {
        return Err(invalid());
    }
    Ok(charge)
}

/// A bounded-memory fold over record lengths. Row order is immaterial; finish
/// validates the relationship between run and step counts. Parent validation
/// remains responsible for per-run identities, fields and original evidence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct JournalTotals {
    positives: u64,
    steps: u64,
    refusals: u64,
    charged: u64,
}

impl JournalTotals {
    /// Failed additions leave every count and charge unchanged.
    pub(super) fn add(&mut self, kind: RecordKind, payload_bytes: u64) -> AppResult<()> {
        let charge = checked_charge(kind, payload_bytes)?;
        let mut next = *self;
        let count = match kind {
            RecordKind::Run => &mut next.positives,
            RecordKind::Step => &mut next.steps,
            RecordKind::Refusal => &mut next.refusals,
        };
        *count = count.checked_add(1).ok_or_else(invalid)?;
        sqlite_total(*count)?;
        next.charged = next.charged.checked_add(charge).ok_or_else(invalid)?;
        sqlite_total(next.charged)?;
        *self = next;
        Ok(())
    }

    pub(super) fn finish(self) -> AppResult<(i64, i64, i64)> {
        let positives = sqlite_total(self.positives)?;
        sqlite_total(self.steps)?;
        let refusals = sqlite_total(self.refusals)?;
        let charged = sqlite_total(self.charged)?;

        // Wide intermediates preserve legal SQLite totals even where the
        // theoretical worst-case ceiling is larger than u64 or SQLite INTEGER.
        let positives_wide = u128::from(self.positives);
        let steps_wide = u128::from(self.steps);
        let refusals_wide = u128::from(self.refusals);
        let minimum = positives_wide * u128::from(RUN_OVERHEAD_BYTES)
            + steps_wide * u128::from(STEP_OVERHEAD_BYTES)
            + refusals_wide * u128::from(REFUSAL_OVERHEAD_BYTES);
        let maximum = positives_wide * u128::from(MAX_POSITIVE_CHARGE_BYTES)
            + refusals_wide * u128::from(MAX_REFUSAL_CHARGE_BYTES);
        if steps_wide > positives_wide * u128::from(MAX_STEPS)
            || u128::from(self.charged) < minimum
            || u128::from(self.charged) > maximum
        {
            return Err(invalid());
        }
        Ok((positives, refusals, charged))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};

    #[test]
    fn journal_accounting_matches_sqlite_utf8_concat_for_escaped_json() {
        let connection = Connection::open_in_memory().unwrap();
        let id = "00000000-0000-4000-8000-000000000001";
        let digest = "d".repeat(64);
        let observations = serde_json::to_string(&serde_json::json!({
            "text": "quoted \"value\"\nCafé 雪 🦀", "backslash": "\\"
        }))
        .unwrap();
        let terminal = serde_json::to_string(&serde_json::json!({"note": "終わり"})).unwrap();
        assert!(observations.len() > observations.chars().count());
        for terminal in [None, Some(terminal.as_str())] {
            let run: i64 = connection.query_row(
                "SELECT 512+length(CAST(?1||?1||?1||?1||?2||'terminal'||?3||coalesce(?4,'') AS BLOB))",
                params![id, digest, observations, terminal],
                |r| r.get(0),
            ).unwrap();
            let payload = 4 * id.len()
                + digest.len()
                + "terminal".len()
                + observations.len()
                + terminal.unwrap_or("").len();
            assert_eq!(
                checked_charge(RecordKind::Run, payload as u64).unwrap(),
                run as u64
            );
        }
        for outcome in [None, Some(terminal.as_str())] {
            let step: i64 = connection
                .query_row(
                    "SELECT 256+length(CAST(coalesce(?1,'')||?2 AS BLOB))",
                    params![outcome, observations],
                    |r| r.get(0),
                )
                .unwrap();
            let payload = outcome.unwrap_or("").len() + observations.len();
            assert_eq!(
                checked_charge(RecordKind::Step, payload as u64).unwrap(),
                step as u64
            );
        }
        let code = "connectionClosed";
        let refusal: i64 = connection
            .query_row(
                "SELECT 256+length(CAST(?1||?1||?2||?3 AS BLOB))",
                params![id, digest, code],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            checked_charge(
                RecordKind::Refusal,
                (2 * id.len() + digest.len() + code.len()) as u64
            )
            .unwrap(),
            refusal as u64
        );
    }

    #[test]
    fn journal_accounting_empty_and_mixed_folds_are_order_independent() {
        assert_eq!(JournalTotals::default().finish().unwrap(), (0, 0, 0));
        let rows = [
            (RecordKind::Step, 7),
            (RecordKind::Run, 21),
            (RecordKind::Refusal, 9),
        ];
        let mut forward = JournalTotals::default();
        let mut reverse = JournalTotals::default();
        for (kind, payload) in rows {
            forward.add(kind, payload).unwrap();
        }
        for (kind, payload) in rows.into_iter().rev() {
            reverse.add(kind, payload).unwrap();
        }
        assert_eq!(forward, reverse);
        assert_eq!(
            forward.finish().unwrap(),
            (1, 1, 512 + 256 + 256 + 7 + 21 + 9)
        );
    }

    #[test]
    fn journal_accounting_all_100_maximum_units_and_refusal_fit_exactly() {
        assert_eq!(MAX_POSITIVE_CHARGE_BYTES, 3_335_896);
        assert_eq!(MAX_REFUSAL_CHARGE_BYTES, 456);
        let mut totals = JournalTotals::default();
        totals.add(RecordKind::Run, MAX_RUN_PAYLOAD_BYTES).unwrap();
        for _ in 0..MAX_STEPS {
            totals
                .add(RecordKind::Step, MAX_STEP_PAYLOAD_BYTES)
                .unwrap();
        }
        totals
            .add(RecordKind::Refusal, MAX_REFUSAL_PAYLOAD_BYTES)
            .unwrap();
        assert_eq!(totals.finish().unwrap(), (1, 1, 3_336_352));
    }

    #[test]
    fn journal_accounting_rejects_each_oversized_record_without_mutation() {
        let mut totals = JournalTotals::default();
        totals.add(RecordKind::Run, 42).unwrap();
        for (kind, maximum) in [
            (RecordKind::Run, MAX_RUN_PAYLOAD_BYTES),
            (RecordKind::Step, MAX_STEP_PAYLOAD_BYTES),
            (RecordKind::Refusal, MAX_REFUSAL_PAYLOAD_BYTES),
        ] {
            let original = totals;
            assert!(totals.add(kind, maximum + 1).is_err());
            assert_eq!(totals, original);
            assert!(totals.add(kind, u64::MAX).is_err());
            assert_eq!(totals, original);
        }
        totals.add(RecordKind::Refusal, 1).unwrap();
        assert_eq!(totals.finish().unwrap(), (1, 1, 512 + 42 + 256 + 1));
    }

    #[test]
    fn journal_accounting_rejects_excess_tiny_steps_and_orphan_steps() {
        let mut totals = JournalTotals::default();
        totals.add(RecordKind::Step, 0).unwrap();
        assert!(totals.finish().is_err());
        totals.add(RecordKind::Run, 0).unwrap();
        for _ in 1..=MAX_STEPS {
            totals.add(RecordKind::Step, 0).unwrap();
        }
        assert!(totals.finish().is_err());
    }

    #[test]
    fn journal_accounting_sqlite_and_accumulator_overflow_are_checked_atomically() {
        assert_eq!(sqlite_total(i64::MAX as u64).unwrap(), i64::MAX);
        assert!(sqlite_total(i64::MAX as u64 + 1).is_err());
        assert!(sqlite_total(u64::MAX).is_err());
        for mut totals in [
            JournalTotals {
                positives: u64::MAX,
                ..JournalTotals::default()
            },
            JournalTotals {
                positives: i64::MAX as u64,
                ..JournalTotals::default()
            },
            JournalTotals {
                charged: u64::MAX,
                ..JournalTotals::default()
            },
            JournalTotals {
                charged: i64::MAX as u64,
                ..JournalTotals::default()
            },
        ] {
            let original = totals;
            assert!(totals.add(RecordKind::Run, 0).is_err());
            assert_eq!(totals, original);
        }
    }

    #[test]
    fn journal_accounting_rejects_malformed_aggregate_bounds() {
        for totals in [
            JournalTotals {
                charged: 1,
                ..JournalTotals::default()
            },
            JournalTotals {
                positives: 1,
                charged: RUN_OVERHEAD_BYTES - 1,
                ..JournalTotals::default()
            },
            JournalTotals {
                positives: 1,
                charged: MAX_POSITIVE_CHARGE_BYTES + 1,
                ..JournalTotals::default()
            },
            JournalTotals {
                refusals: 1,
                charged: MAX_REFUSAL_CHARGE_BYTES + 1,
                ..JournalTotals::default()
            },
            JournalTotals {
                positives: 1,
                steps: 1,
                charged: RUN_OVERHEAD_BYTES + STEP_OVERHEAD_BYTES - 1,
                ..JournalTotals::default()
            },
        ] {
            assert!(totals.finish().is_err());
        }
    }

    #[test]
    fn journal_accounting_large_counts_do_not_overflow_worst_case_intermediates() {
        let positives = i64::MAX as u64 / RUN_OVERHEAD_BYTES;
        let charged = positives * RUN_OVERHEAD_BYTES;
        let totals = JournalTotals {
            positives,
            charged,
            ..JournalTotals::default()
        };
        assert_eq!(
            totals.finish().unwrap(),
            (positives as i64, 0, charged as i64)
        );
    }
}
