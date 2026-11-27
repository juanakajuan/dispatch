use super::*;
use crate::ui::text::fit;

fn ts(secs: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(secs).expect("valid timestamp")
}

#[test]
fn age_buckets() {
    let now = ts(10_000_000);
    for (secs, want) in [
        (0, "now"),
        (59, "now"),
        (60, "1m"),
        (3599, "59m"),
        (3600, "1h"),
        (86_399, "23h"),
        (86_400, "1d"),
        (9 * 86_400, "9d"),
        (40 * 86_400, "1mo"),
        (400 * 86_400, "1y"),
    ] {
        assert_eq!(age(ts(10_000_000 - secs), now), want, "{secs}s");
    }
    assert_eq!(age(ts(10_000_100), now), "now", "a clock skew never prints a negative age");
}

#[test]
fn fit_cuts_by_cells() {
    assert_eq!(fit("hello", 10), "hello");
    assert_eq!(fit("hello world", 6), "hello…");
    assert_eq!(fit("anything", 0), "");
}

#[test]
fn wrap_breaks_at_spaces_and_splits_a_word_wider_than_the_row() {
    assert_eq!(wrap("git worktree add", 8), ["git", "worktree", "add"]);
    assert_eq!(wrap("a  b c", 3), ["a b", "c"]);
    assert_eq!(wrap("ab /very/long/path", 6), ["ab", "/very/", "long/p", "ath"]);
    assert!(wrap("", 4).is_empty());
}
