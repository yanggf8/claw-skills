//! Deciding which issue date this run is for.

/// The upcoming Sunday in Asia/Taipei, or today when today is Sunday.
///
/// `now` is injectable so the boundary cases are testable rather than
/// whatever-day-CI-runs.
///
/// The Python did this by adding 8 hours to a UTC timestamp and taking the
/// date. That is right for Taipei, which has had no DST since 1979, and it is
/// kept as a real timezone lookup here anyway — the arithmetic version quietly
/// becomes wrong for any zone that does observe DST, and a reader copying it
/// elsewhere would not be warned.
pub fn next_sunday_taipei(now: jiff::Timestamp) -> String {
    let today = now
        .in_tz("Asia/Taipei")
        .map(|z| z.date())
        .unwrap_or_else(|_| now.in_tz("UTC").unwrap().date());
    // jiff's weekday: Monday = 1 … Sunday = 7.
    let days_until_sunday = (7 - today.weekday().to_monday_one_offset()) % 7;
    (today + jiff::Span::new().days(days_until_sunday as i64))
        .strftime("%Y-%m-%d")
        .to_string()
}
