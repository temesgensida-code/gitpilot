//! Application state and input handling. The UI never calls git directly:
//! it asks the app, and the app runs git on worker threads and reports back
//! through a channel. That keeps the event loop responsive while large diffs
//! or slow operations run.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::Sender,
    thread,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::git::{self, Preview, Side, Snapshot};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Status,
    Log,
    Branches,
    Stash,
    Conflicts,
}

impl View {
    pub const ALL: [View; 5] = [
        View::Status,
        View::Log,
        View::Branches,
        View::Stash,
        View::Conflicts,
    ];

    pub fn title(self) -> &'static str {
        match self {
            View::Status => "Status",
            View::Log => "Log",
            View::Branches => "Branches",
            View::Stash => "Stash",
            View::Conflicts => "Conflicts",
        }
    }
}

/// Messages from worker threads back to the event loop.
pub enum Msg {
    Snapshot(Result<Snapshot, String>),
    Preview { key: String, text: String },
    Action(Result<String, String>),
}

/// A line of text the user is typing in the footer.
pub enum Input {
    Commit(String),
    Stash(String),
}

enum Target {
    Git(Preview),
    Text(String),
}

pub struct App {
    pub workdir: PathBuf,
    tx: Sender<Msg>,
    pub view: View,
    pub snap: Snapshot,
    pub list: ListState,
    pub preview: String,
    pub preview_scroll: u16,
    preview_key: String,
    pub input: Option<Input>,
    pub notice: String,
    pub notice_is_error: bool,
    /// Number of background jobs still running (drives the footer spinner).
    pub pending: usize,
    pub quit: bool,
}

impl App {
    pub fn new(workdir: PathBuf, tx: Sender<Msg>) -> Self {
        Self {
            workdir,
            tx,
            view: View::Status,
            snap: Snapshot::default(),
            list: ListState::default(),
            preview: String::new(),
            preview_scroll: 0,
            preview_key: String::new(),
            input: None,
            notice: "loading…".into(),
            notice_is_error: false,
            pending: 0,
            quit: false,
        }
    }

    fn spawn<F>(&mut self, job: F)
    where
        F: FnOnce() -> Msg + Send + 'static,
    {
        self.pending += 1;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(job());
        });
    }

    pub fn refresh(&mut self) {
        let dir = self.workdir.clone();
        self.spawn(move || Msg::Snapshot(git::load_snapshot(&dir).map_err(|e| format!("{e:#}"))));
    }

    /// Runs a mutating git operation off-thread, then reloads the snapshot.
    fn action<F>(&mut self, op: F)
    where
        F: FnOnce(&Path) -> git::Result<String> + Send + 'static,
    {
        let dir = self.workdir.clone();
        self.spawn(move || Msg::Action(op(&dir).map_err(|e| format!("{e:#}"))));
    }

    pub fn on_msg(&mut self, msg: Msg) {
        self.pending = self.pending.saturating_sub(1);
        match msg {
            Msg::Snapshot(Ok(snap)) => {
                self.snap = snap;
                self.clamp_selection();
                self.request_preview();
            }
            Msg::Snapshot(Err(e)) => self.set_notice(e, true),
            Msg::Preview { key, text } => {
                // Ignore results for a selection the user has already left.
                if key == self.preview_key {
                    self.preview = text;
                }
            }
            Msg::Action(Ok(note)) => {
                self.set_notice(note, false);
                self.refresh();
            }
            Msg::Action(Err(e)) => {
                self.set_notice(e, true);
                self.refresh();
            }
        }
    }

    fn set_notice(&mut self, text: String, is_error: bool) {
        self.notice = text;
        self.notice_is_error = is_error;
    }

    fn item_count(&self) -> usize {
        match self.view {
            View::Status => self.snap.files.len(),
            View::Log => self.snap.commits.len(),
            View::Branches => self.snap.branches.len(),
            View::Stash => self.snap.stashes.len(),
            View::Conflicts => self.snap.conflicts.len(),
        }
    }

    fn clamp_selection(&mut self) {
        let len = self.item_count();
        let selected = match self.list.selected() {
            _ if len == 0 => None,
            Some(i) => Some(i.min(len - 1)),
            None => Some(0),
        };
        self.list.select(selected);
    }

    fn target(&self) -> Target {
        let Some(i) = self.list.selected() else {
            return Target::Text(String::new());
        };
        let found = match self.view {
            View::Status => self.snap.files.get(i).map(|f| {
                Target::Git(Preview::Diff {
                    path: f.path.clone(),
                    staged: f.staged,
                })
            }),
            View::Log => self
                .snap
                .commits
                .get(i)
                .map(|c| Target::Git(Preview::Commit(c.full.clone()))),
            View::Branches => self
                .snap
                .branches
                .get(i)
                .map(|b| Target::Text(format!("branch {}\n\ntip: {}", b.name, b.tip))),
            View::Stash => self
                .snap
                .stashes
                .get(i)
                .map(|s| Target::Git(Preview::Commit(s.oid.clone()))),
            View::Conflicts => self
                .snap
                .conflicts
                .get(i)
                .map(|p| Target::Git(Preview::Conflict(p.clone()))),
        };
        found.unwrap_or_else(|| Target::Text(String::new()))
    }

    /// Refreshes the preview for the current selection. Git-backed previews
    /// are computed on a worker thread; the result is matched by key.
    pub fn request_preview(&mut self) {
        let key = format!("{:?}:{:?}", self.view, self.list.selected());
        if key != self.preview_key {
            self.preview_key = key.clone();
            self.preview_scroll = 0;
        }
        match self.target() {
            Target::Text(text) => self.preview = text,
            Target::Git(spec) => {
                let dir = self.workdir.clone();
                self.spawn(move || {
                    let text =
                        git::preview(&dir, &spec).unwrap_or_else(|e| format!("error: {e:#}"));
                    Msg::Preview { key, text }
                });
            }
        }
    }

    fn switch_view(&mut self, view: View) {
        self.view = view;
        self.list
            .select(if self.item_count() > 0 { Some(0) } else { None });
        self.request_preview();
    }

    fn step(&mut self, delta: isize) {
        let len = self.item_count();
        if len == 0 {
            return;
        }
        let current = self.list.selected().unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, len as isize - 1) as usize;
        self.list.select(Some(next));
        self.request_preview();
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if self.input.is_some() {
            self.on_input_key(key.code);
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char(c @ '1'..='5') => {
                let idx = c as usize - '1' as usize;
                self.switch_view(View::ALL[idx]);
            }
            KeyCode::Tab => {
                let idx = View::ALL.iter().position(|v| *v == self.view).unwrap_or(0);
                self.switch_view(View::ALL[(idx + 1) % View::ALL.len()]);
            }
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::PageDown => self.preview_scroll = self.preview_scroll.saturating_add(10),
            KeyCode::PageUp => self.preview_scroll = self.preview_scroll.saturating_sub(10),
            code => self.view_key(code),
        }
    }

    /// Keys that only mean something in one view.
    fn view_key(&mut self, code: KeyCode) {
        let selected = self.list.selected();
        match (self.view, code) {
            (View::Status, KeyCode::Char(' ')) => {
                if let Some(f) = selected.and_then(|i| self.snap.files.get(i)) {
                    let (path, staged) = (f.path.clone(), f.staged);
                    if staged {
                        self.action(move |d| git::unstage(d, &path));
                    } else {
                        self.action(move |d| git::stage(d, &path));
                    }
                }
            }
            (View::Status, KeyCode::Char('a')) => self.action(git::stage_all),
            (View::Status, KeyCode::Char('c')) => self.input = Some(Input::Commit(String::new())),
            (View::Status, KeyCode::Char('S')) => self.input = Some(Input::Stash(String::new())),

            (View::Log, KeyCode::Char('p')) => {
                if let Some(c) = selected.and_then(|i| self.snap.commits.get(i)) {
                    let oid = c.full.clone();
                    self.action(move |d| git::cherry_pick(d, &oid));
                }
            }

            (View::Branches, KeyCode::Enter) => {
                if let Some(b) = selected.and_then(|i| self.snap.branches.get(i)) {
                    let name = b.name.clone();
                    self.action(move |d| git::checkout(d, &name));
                }
            }

            (View::Stash, KeyCode::Enter | KeyCode::Char('a')) => {
                if let Some(s) = selected.and_then(|i| self.snap.stashes.get(i)) {
                    let idx = s.index;
                    self.action(move |d| git::stash_apply(d, idx));
                }
            }
            (View::Stash, KeyCode::Char('p')) => {
                if let Some(s) = selected.and_then(|i| self.snap.stashes.get(i)) {
                    let idx = s.index;
                    self.action(move |d| git::stash_pop(d, idx));
                }
            }
            (View::Stash, KeyCode::Char('d')) => {
                if let Some(s) = selected.and_then(|i| self.snap.stashes.get(i)) {
                    let idx = s.index;
                    self.action(move |d| git::stash_drop(d, idx));
                }
            }

            (View::Conflicts, KeyCode::Char('o')) => {
                if let Some(p) = selected.and_then(|i| self.snap.conflicts.get(i)) {
                    let path = p.clone();
                    self.action(move |d| git::take_side(d, &path, Side::Ours));
                }
            }
            (View::Conflicts, KeyCode::Char('t')) => {
                if let Some(p) = selected.and_then(|i| self.snap.conflicts.get(i)) {
                    let path = p.clone();
                    self.action(move |d| git::take_side(d, &path, Side::Theirs));
                }
            }
            (View::Conflicts, KeyCode::Char('m')) => {
                if let Some(p) = selected.and_then(|i| self.snap.conflicts.get(i)) {
                    let path = p.clone();
                    self.action(move |d| git::mark_resolved(d, &path));
                }
            }
            _ => {}
        }
    }

    fn input_buf(&mut self) -> Option<&mut String> {
        match self.input.as_mut()? {
            Input::Commit(buf) | Input::Stash(buf) => Some(buf),
        }
    }

    fn on_input_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.input = None,
            KeyCode::Enter => self.submit_input(),
            KeyCode::Backspace => {
                if let Some(buf) = self.input_buf() {
                    buf.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(buf) = self.input_buf() {
                    buf.push(c);
                }
            }
            _ => {}
        }
    }

    fn submit_input(&mut self) {
        match self.input.take() {
            Some(Input::Commit(msg)) if msg.trim().is_empty() => {
                self.input = Some(Input::Commit(msg));
                self.set_notice("commit message is empty".into(), true);
            }
            Some(Input::Commit(msg)) => {
                self.action(move |d| git::commit(d, &msg));
            }
            Some(Input::Stash(msg)) => {
                let msg = if msg.trim().is_empty() {
                    "gitpilot stash".to_string()
                } else {
                    msg
                };
                self.action(move |d| git::stash_save(d, &msg));
            }
            None => {}
        }
    }
}
