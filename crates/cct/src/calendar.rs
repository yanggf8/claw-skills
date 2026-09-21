//! ET trading calendar shared by the trigger gate and the watchdog.
//!
//! One table, two consumers: `cct-trigger` refuses to run off-session, and
//! the watchdog's backward check knows which days to excuse. The table lives
//! here and nowhere else — a second copy of the dates would be a second
//! place to drift, which is the same rule that keeps the read schedule in
//! cron.db instead of in a source file.

use jiff::civil::{Date, Weekday};

use crate::freshness::is_weekend;

/// NYSE closures, 2026 only. A year without an entry is not "no holidays":
/// both consumers say so explicitly instead of guessing — the watchdog
/// appends "holiday table has no data for YYYY" to a missing-run finding,
/// and the gate lets an unknown-year weekday through like any trading day.
pub fn holidays(year: i16) -> Option<&'static [Date]> {
    const HOLIDAYS_2026: [Date; 10] = [
        jiff::civil::date(2026, 1, 1),  // New Year's Day
        jiff::civil::date(2026, 1, 19), // MLK Day
        jiff::civil::date(2026, 2, 16), // Presidents Day
        jiff::civil::date(2026, 4, 3),  // Good Friday
        jiff::civil::date(2026, 5, 25), // Memorial Day
        jiff::civil::date(2026, 6, 19), // Juneteenth
        jiff::civil::date(2026, 7, 3), // Independence Day (observed, Jul 4 is a Saturday)
        jiff::civil::date(2026, 9, 7),  // Labor Day
        jiff::civil::date(2026, 11, 26), // Thanksgiving
        jiff::civil::date(2026, 12, 25), // Christmas
    ];
    match year {
        2026 => Some(&HOLIDAYS_2026),
        _ => None,
    }
}

/// A NYSE session day: Mon-Fri and not in the holiday table.
///
/// An unknown year cannot be claimed holiday-free, so its weekdays count as
/// trading days and the callers surface the uncertainty themselves.
pub fn is_trading_day(day: Date) -> bool {
    if is_weekend(day) {
        return false;
    }
    match holidays(day.year()) {
        Some(list) => !list.contains(&day),
        None => true,
    }
}

/// The most recent trading day strictly before `day`.
pub fn prev_trading_day(day: Date) -> Date {
    use jiff::ToSpan;
    let mut d = day - 1.day();
    while !is_trading_day(d) {
        d -= 1.day();
    }
    d
}

/// `%a`-style abbreviation, for watchdog lines that name a day.
pub fn weekday_short(w: Weekday) -> &'static str {
    match w {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    }
}
