//! Bounded exhaustive check of the real reducer against jobs that finish
//! late and in any order. The checked promise: dispatch never sends a
//! `WriteJob::Start` for a session key that already has a session, or that
//! already has a start in flight. The run tries every sequence of the events
//! in `Event`, up to `DEPTH` events, from the `app()` fixture, and drops a
//! sequence that reaches a state an earlier sequence reached.

mod world;

use super::*;
use std::collections::HashSet;
use world::{World, write_summary};

/// The longest event sequence the check tries. A start that skips
/// `fixer`'s check of `starting` takes 9 events to show, the most of any
/// break this check was shown to catch.
const DEPTH: usize = 9;

/// One thing that can happen next: a user action, or a job or the outside
/// world moving on.
#[derive(Clone, Copy, Debug)]
enum Event {
    Key(char),
    /// Ctrl-S in the start dialog.
    Confirm,
    /// Esc in the start dialog.
    Cancel,
    ClickTab(Tab),
    ClickRow(RowKey),
    /// The writer thread runs its next job.
    Write,
    /// The read job at this index of `World::reads` finishes.
    Read(usize),
    Tick,
    /// The agent of a handed-off PR merges it on GitHub.
    AgentMerges(PrNumber),
    /// An open PR starts to conflict with its base on GitHub.
    Conflicts(PrNumber),
}

/// The keys the user may press in the list.
const LIST_KEYS: [char; 4] = ['m', 's', 'j', 'k'];

/// A broken promise: dispatch sent a start for `key` on `branch` while `key`
/// already had a session or a start in flight.
struct SecondStart {
    key: SessionKey,
    branch: BranchName,
}

impl std::fmt::Display for SecondStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let SecondStart { key, branch } = self;
        write!(
            f,
            "dispatch sends a second start for {key} on branch {}, while {key} already has a session or a start in flight.",
            branch.as_str()
        )
    }
}

struct Run {
    state: State,
    world: World,
    clock: Instant,
    /// What each event did, in the words a counterexample uses.
    story: Vec<String>,
}

impl Run {
    fn new() -> Self {
        let (state, clock) = app();
        let world = World::new(state.prs.items.clone());
        Run { state, world, clock, story: Vec::new() }
    }

    fn replay(trace: &[Event]) -> Self {
        let mut run = Run::new();
        for &event in trace {
            assert!(run.step(event).is_ok(), "a prefix of an explored trace keeps the promise");
        }
        run
    }

    /// Draws like the shell does, so clicks hit what is on screen.
    fn draw(&mut self) {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).expect("terminal");
        let mut hits = HitMap::default();
        terminal.draw(|f| hits = crate::ui::draw(f, &self.state)).expect("draw");
        self.state.hits = hits;
    }

    fn click(&mut self, target: HitTarget) -> Msg {
        self.draw();
        let (rect, _) = self.state.hits.targets.iter().find(|(_, t)| *t == target).expect("the target is drawn");
        click(rect.x, rect.y, self.clock)
    }

    /// Applies `event`, and says how it broke the promise, if it did.
    fn step(&mut self, event: Event) -> Result<(), SecondStart> {
        let (msg, said) = match event {
            Event::Key(c) => (key(c), format!("The user presses {c}.")),
            Event::Confirm => (ctrl('s'), "The user confirms the start dialog.".to_string()),
            Event::Cancel => (code(KeyCode::Esc), "The user closes the start dialog.".to_string()),
            Event::ClickTab(tab) => (self.click(HitTarget::Tab(tab)), format!("The user clicks the {tab:?} tab.")),
            Event::ClickRow(row) => (self.click(HitTarget::Row(row)), format!("The user clicks the row {row:?}.")),
            Event::Write => {
                let job = write_summary(self.world.writes.front().expect("a write job is queued"));
                (self.world.run_write(), format!("The writer runs {job}."))
            }
            Event::Read(i) => {
                let said = format!("The read job {:?} finishes.", self.world.reads[i]);
                (self.world.run_read(i, self.clock), said)
            }
            Event::Tick => {
                self.clock += Duration::from_hours(1);
                (Msg::Tick(self.clock), "An hour passes.".to_string())
            }
            Event::AgentMerges(n) => {
                self.world.merge_on_github(n);
                (Msg::FocusGained, format!("The agent merges PR {n} on GitHub."))
            }
            Event::Conflicts(n) => {
                self.world.conflict_on_github(n);
                (Msg::FocusGained, format!("PR {n} starts to conflict with its base on GitHub."))
            }
        };
        self.clock += Duration::from_secs(1);
        let busy: BTreeSet<SessionKey> = self.state.sessions.keys().chain(&self.state.starting).copied().collect();
        let cmds = update(&mut self.state, msg);
        self.story.push(said);
        let broken = starts(&cmds)
            .into_iter()
            .find(|req| busy.contains(&req.key))
            .map(|req| SecondStart { key: req.key, branch: req.branch.clone() });
        self.world.take(cmds);
        broken.map_or(Ok(()), Err)
    }

    /// Every event that can happen next.
    fn next_events(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        match &self.state.focus {
            Focus::List => {
                events.extend(LIST_KEYS.map(Event::Key));
                events.extend([Tab::Issues, Tab::Prs, Tab::Sessions].map(Event::ClickTab));
                self.draw();
                events.extend(self.state.hits.targets.iter().filter_map(|(_, t)| match t {
                    HitTarget::Row(row) => Some(Event::ClickRow(*row)),
                    _ => None,
                }));
            }
            Focus::Dialog(_) => events.extend([Event::Confirm, Event::Cancel]),
            _ => {}
        }
        if !self.world.writes.is_empty() {
            events.push(Event::Write);
        }
        events.extend((0..self.world.reads.len()).map(Event::Read));
        events.push(Event::Tick);
        let open = |n: &&PrNumber| self.world.prs.iter().any(|p| p.number == **n && p.state == PrState::Open);
        events.extend(self.state.handed_off.keys().filter(open).copied().map(Event::AgentMerges));
        events.extend(
            self.world
                .prs
                .iter()
                .filter(|p| p.state == PrState::Open && p.mergeability != Mergeability::Conflict)
                .map(|p| Event::Conflicts(p.number)),
        );
        events
    }

    /// The state that decides what can happen next. Two runs with the same
    /// fingerprint have the same futures.
    fn fingerprint(&self) -> String {
        let s = &self.state;
        let focus = match &s.focus {
            Focus::List => "list".to_string(),
            Focus::Dialog(d) => format!("dialog {}", d.key),
            _ => "other".to_string(),
        };
        let sessions: Vec<_> = s.sessions.values().map(|x| (x.key, x.branch.as_str(), x.state)).collect();
        let prs: Vec<_> = s.prs.items.iter().map(|p| (p.number, p.state, p.mergeability)).collect();
        let polls = (s.issues.poll.in_flight(), s.prs.poll.in_flight(), s.observe.in_flight(), s.due.github_soon);
        format!(
            "{:?}|{:?}|{focus}|{:?}|{:?}|{sessions:?}|{:?}|{:?}|{prs:?}|{polls:?}|{}",
            s.tab,
            selected_row(s),
            s.armed,
            s.starting,
            s.handed_off,
            s.marked,
            self.world.fingerprint()
        )
    }
}

/// Explores breadth first, so a counterexample is a shortest one.
fn explore(depth: usize) -> Result<usize, Vec<String>> {
    let mut seen = HashSet::from([Run::new().fingerprint()]);
    let mut frontier: Vec<Vec<Event>> = vec![Vec::new()];
    for _ in 0..depth {
        let mut next = Vec::new();
        for trace in &frontier {
            for event in Run::replay(trace).next_events() {
                let mut run = Run::replay(trace);
                if let Err(broken) = run.step(event) {
                    run.story.push(broken.to_string());
                    return Err(run.story);
                }
                if seen.insert(run.fingerprint()) {
                    next.push([trace.as_slice(), &[event]].concat());
                }
            }
        }
        frontier = next;
    }
    Ok(seen.len())
}

#[test]
#[ignore = "slow; run by the reducer-races skill: cargo test --release races -- --ignored"]
fn a_start_never_races_another_start_of_its_key() {
    match explore(DEPTH) {
        Ok(states) => eprintln!("checked {states} states, every sequence of up to {DEPTH} events"),
        Err(story) => panic!("counterexample:\n{}", story.join("\n")),
    }
}
