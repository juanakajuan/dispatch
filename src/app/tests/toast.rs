use super::*;

#[test]
fn the_style_takes_sentences_and_work_in_progress() {
    for text in ["#16 has no changes.", "PR #10 is a draft. Press m again to merge it.", "Starting #16…"] {
        assert!(follows_style(text), "{text}");
    }
}

#[test]
fn the_style_refuses_lower_case_a_missing_stop_and_a_double_stop() {
    for text in ["", "attach failed: boom.", "Stopped #16", "Saving settings failed: disk full.."] {
        assert!(!follows_style(text), "{text:?}");
    }
}

#[test]
fn a_failure_ends_with_one_full_stop() {
    assert_eq!(failed("Starting #16", "tmux exited"), "Starting #16 failed: tmux exited.");
    assert_eq!(failed("Starting #16", "x exists. Move it away."), "Starting #16 failed: x exists. Move it away.");
}

#[test]
#[should_panic(expected = "toast breaks the style")]
fn a_toast_that_breaks_the_style_fails_the_tests() {
    let (mut state, _) = app();
    info(&mut state, "sessions: boom".to_string());
}
