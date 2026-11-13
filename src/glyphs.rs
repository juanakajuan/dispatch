//! Every glyph that marks a state. `Icons` names each mark once, so a new
//! mark must exist in both sets. Each glyph is one cell wide, so both sets
//! give the same layout.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconSet {
    #[default]
    Unicode,
    NerdFont,
}

pub struct Icons {
    pub running: &'static str,
    pub needs_you: &'static str,
    pub exited: &'static str,
    pub starting: &'static str,
    pub check_pass: &'static str,
    pub check_fail: &'static str,
    pub check_pending: &'static str,
    pub check_none: &'static str,
    pub pr_open: &'static str,
    pub pr_draft: &'static str,
    pub pr_merged: &'static str,
    pub pr_closed: &'static str,
    /// A row with no state to show.
    pub idle: &'static str,
    pub radio_on: &'static str,
    pub radio_off: &'static str,
}

pub const UNICODE: Icons = Icons {
    running: "●",
    needs_you: "▲",
    exited: "○",
    starting: "…",
    check_pass: "✓",
    check_fail: "✗",
    check_pending: "●",
    check_none: "·",
    pr_open: "●",
    pr_draft: "◌",
    pr_merged: "✓",
    pr_closed: "✗",
    idle: "·",
    radio_on: "●",
    radio_off: "○",
};

pub const NERD_FONT: Icons = Icons {
    running: "\u{f444}",       // nf-oct-dot_fill
    needs_you: "\u{f421}",     // nf-oct-alert
    exited: "\u{f4aa}",        // nf-oct-circle
    starting: "\u{f4e3}",      // nf-oct-hourglass
    check_pass: "\u{f42e}",    // nf-oct-check
    check_fail: "\u{f467}",    // nf-oct-x
    check_pending: "\u{f444}", // nf-oct-dot_fill
    check_none: "\u{f48b}",    // nf-oct-dash
    pr_open: "\u{f407}",       // nf-oct-git_pull_request
    pr_draft: "\u{f4dd}",      // nf-oct-git_pull_request_draft
    pr_merged: "\u{f419}",     // nf-oct-git_merge
    pr_closed: "\u{f4dc}",     // nf-oct-git_pull_request_closed
    idle: "\u{f48b}",          // nf-oct-dash
    radio_on: "\u{f043e}",     // nf-md-radiobox_marked
    radio_off: "\u{f043d}",    // nf-md-radiobox_blank
};

impl IconSet {
    pub fn next(self) -> IconSet {
        match self {
            IconSet::Unicode => IconSet::NerdFont,
            IconSet::NerdFont => IconSet::Unicode,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            IconSet::Unicode => "unicode",
            IconSet::NerdFont => "nerdfont",
        }
    }

    pub fn icons(self) -> &'static Icons {
        match self {
            IconSet::Unicode => &UNICODE,
            IconSet::NerdFont => &NERD_FONT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_in_both_sets_is_one_cell() {
        for set in [IconSet::Unicode, IconSet::NerdFont] {
            let i = set.icons();
            for g in [
                i.running,
                i.needs_you,
                i.exited,
                i.starting,
                i.check_pass,
                i.check_fail,
                i.check_pending,
                i.check_none,
                i.pr_open,
                i.pr_draft,
                i.pr_merged,
                i.pr_closed,
                i.idle,
                i.radio_on,
                i.radio_off,
            ] {
                assert_eq!(crate::ui::cells(g), 1, "{set:?} glyph {g:?}");
            }
        }
    }
}
