//! Reading persona-core's issue listing.

/// Pull `status=` for one issue out of `streams issues list` output.
///
/// The listing is one issue per line, `id=… status=… …`. Matching on
/// `"id={issue_id} "` with the trailing space matters: without it `id=abc`
/// prefix-matches `id=abcdef`.
pub fn status_of(listing: &str, issue_id: &str) -> Option<String> {
    let needle = format!("id={issue_id} ");
    listing
        .lines()
        .find(|line| line.starts_with(&needle))?
        .split_whitespace()
        .find_map(|tok| tok.strip_prefix("status=").map(str::to_string))
}

/// An issue in one of these states needs no work this run.
pub fn already_done(status: Option<&str>) -> bool {
    matches!(status, Some("published") | Some("delivered"))
}
