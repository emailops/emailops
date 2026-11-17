//! When a scheduled copy is due: the pure decision behind the check the app
//! runs every few minutes while it is open.

use chrono::{DateTime, NaiveDate, TimeDelta, TimeZone};

use super::BackupFrequency;

/// The scheduled moment at or before `now`: today at `hour`:00 local time once
/// that has passed, yesterday's before it. `None` only for an hour outside
/// 0–23 or a date at the edge of chrono's range.
pub fn last_slot<Tz: TimeZone>(now: &DateTime<Tz>, hour: u32) -> Option<DateTime<Tz>> {
    let tz = now.timezone();
    let today = now.date_naive();
    match slot_on(&tz, today, hour) {
        Some(slot) if slot <= *now => Some(slot),
        _ => slot_on(&tz, today.pred_opt()?, hour),
    }
}

/// `hour`:00 on `date`, local time. An hour the clocks skip (the start of
/// daylight saving time) counts from the first moment after it.
fn slot_on<Tz: TimeZone>(tz: &Tz, date: NaiveDate, hour: u32) -> Option<DateTime<Tz>> {
    let naive = date.and_hms_opt(hour, 0, 0)?;
    tz.from_local_datetime(&naive)
        .earliest()
        .or_else(|| tz.from_local_datetime(&(naive + TimeDelta::hours(1))).earliest())
}

/// Whether a copy is due at `now`, given when the newest copy was taken (unix
/// seconds, `None` before the first one).
///
/// Daily: the newest copy predates the last slot. Weekly: it predates the slot
/// six days before that, so a copy taken on a Monday is due again the next
/// Monday. A slot missed while the app was closed or the computer asleep is
/// caught up at the next check. A copy dated after `now` means the clock moved
/// back, so it says nothing about today.
pub fn is_due<Tz: TimeZone>(now: &DateTime<Tz>, hour: u32, frequency: BackupFrequency, newest: Option<i64>) -> bool {
    let days_between = match frequency {
        BackupFrequency::Off => return false,
        BackupFrequency::Daily => 0,
        BackupFrequency::Weekly => 6,
    };
    let Some(newest) = newest else {
        return true;
    };
    if newest > now.timestamp() {
        return true;
    }
    let Some(slot) = last_slot(now, hour) else {
        return false;
    };
    let Some(cutoff) = slot
        .date_naive()
        .checked_sub_days(chrono::Days::new(days_between))
        .and_then(|date| slot_on(&now.timezone(), date, hour))
    else {
        return false;
    };
    newest < cutoff.timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use chrono_tz::Europe::Madrid;

    fn at(day: u32, hour: u32, minute: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(3600)
            .expect("valid offset")
            .with_ymd_and_hms(2026, 11, day, hour, minute, 0)
            .single()
            .expect("valid time")
    }

    #[test]
    fn last_slot_is_today_once_the_hour_has_passed_and_yesterday_before() {
        assert_eq!(last_slot(&at(17, 10, 0), 3), Some(at(17, 3, 0)));
        assert_eq!(last_slot(&at(17, 3, 0), 3), Some(at(17, 3, 0)));
        assert_eq!(last_slot(&at(17, 2, 59), 3), Some(at(16, 3, 0)));
    }

    #[test]
    fn an_hour_the_clocks_skip_counts_from_the_hour_after() {
        // Madrid jumps from 02:00 to 03:00 on 29/03/2026.
        let now = Madrid
            .with_ymd_and_hms(2026, 3, 29, 12, 0, 0)
            .single()
            .expect("valid time");
        let expected = Madrid
            .with_ymd_and_hms(2026, 3, 29, 3, 0, 0)
            .single()
            .expect("valid time");

        assert_eq!(last_slot(&now, 2), Some(expected));
    }

    #[test]
    fn is_due_follows_the_frequency_the_hour_and_the_newest_copy() {
        let ts = |t: DateTime<FixedOffset>| Some(t.timestamp());
        let cases = [
            ("off never copies", at(17, 10, 0), BackupFrequency::Off, None, false),
            ("first copy", at(17, 10, 0), BackupFrequency::Daily, None, true),
            (
                "daily, newest is from before today's slot",
                at(17, 10, 0),
                BackupFrequency::Daily,
                ts(at(16, 3, 2)),
                true,
            ),
            (
                "daily, newest is from today's slot",
                at(17, 10, 0),
                BackupFrequency::Daily,
                ts(at(17, 3, 1)),
                false,
            ),
            (
                "daily, before the hour, newest is from yesterday's slot",
                at(17, 2, 0),
                BackupFrequency::Daily,
                ts(at(16, 3, 1)),
                false,
            ),
            (
                "daily, a missed slot is caught up",
                at(17, 2, 0),
                BackupFrequency::Daily,
                ts(at(15, 3, 1)),
                true,
            ),
            (
                "weekly, six days on",
                at(23, 10, 0),
                BackupFrequency::Weekly,
                ts(at(17, 3, 1)),
                false,
            ),
            (
                "weekly, seven days on",
                at(24, 3, 0),
                BackupFrequency::Weekly,
                ts(at(17, 3, 1)),
                true,
            ),
            (
                "weekly, a copy caught up late is due at the slot a week on",
                at(24, 3, 0),
                BackupFrequency::Weekly,
                ts(at(17, 10, 0)),
                true,
            ),
            (
                "clock moved back",
                at(17, 10, 0),
                BackupFrequency::Daily,
                ts(at(18, 10, 0)),
                true,
            ),
        ];
        for (name, now, frequency, newest, expected) in cases {
            assert_eq!(is_due(&now, 3, frequency, newest), expected, "{name}");
        }
    }
}
