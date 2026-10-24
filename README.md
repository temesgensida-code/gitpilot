# GitPilot 🛫

[![CI](https://github.com/temesgensida-code/gitpilot/actions/workflows/ci.yml/badge.svg)](https://github.com/temesgensida-code/gitpilot/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

> **A terminal Git cockpit**: stage, diff, browse, resolve conflicts, stash, and cherry-pick without memorizing commands.

Built in Rust using [Ratatui](https://github.com/ratatui/ratatui) and [libgit2](https://libgit2.org/).

---

## ⚡ Quick Install

Run this command in your terminal to install GitPilot instantly:

```bash
curl -fsSL https://raw.githubusercontent.com/temesgensida-code/gitpilot/main/install.sh | bash
```

Once installed, navigate to **any** Git repository and launch it with:

```bash
gitpilot
# or
Gitpilot
```

*(The installer creates both `gitpilot` and `Gitpilot` commands in your PATH for convenience).*

### Alternative Installation Options

#### With Cargo
```bash
cargo install --git https://github.com/temesgensida-code/gitpilot
```

#### From Source
```bash
git clone https://github.com/temesgensida-code/gitpilot.git
cd gitpilot
cargo build --release
cp target/release/gitpilot ~/.local/bin/
```

---

## 🎮 Overview & Layout

GitPilot provides a responsive 3-section cockpit:

```
┌──────────────────────────────── GitPilot · main ────────────────────────────────┐
│ 1 Status     2 Log     3 Branches     4 Stash     5 Conflicts                   │
├───────────────────────────────────┬─────────────────────────────────────────────┤
│ ▸ ? untracked  src/config.rs      │ diff --git a/src/config.rs b/src/config.rs  │
│   M unstaged   src/main.rs        │ +++ b/src/config.rs                         │
│   M staged     Cargo.toml         │ @@ -0,0 +1,15 @@                            │
│                                   │ +pub struct Config { ... }                  │
│                                   │                                             │
├───────────────────────────────────┴─────────────────────────────────────────────┤
│ committed 9a4f21d                                                               │
│ 1-5/tab views · j/k move · pgup/pgdn scroll · r refresh · q quit │ space stage  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

1. **Header**: Active branch indicator and numbered view tabs (`1`–`5`).
2. **Cockpit Body**:
   - **Left Panel (40%)**: Selectable list of files, commits, branches, stashes, or conflicts.
   - **Right Panel (60%)**: Live syntax-highlighted diff and preview pane with smooth scrolling.
3. **Footer**:
   - Instant feedback notifications with error highlighting.
   - Animated background worker spinner (`⟳`).
   - Inline prompt for commit and stash messages.
   - Context-sensitive shortcut cheatsheet.

---

## 🧭 The 5 Cockpit Views

| View | Key | Features & Actions |
| :--- | :---: | :--- |
| **Status** | `1` | Inspect staged, unstaged, and untracked changes.<br>• `Space` — Toggle stage/unstage for highlighted file<br>• `a` — Stage all changes (`git add -A`)<br>• `c` — Commit staged changes with inline message prompt<br>• `S` — Stash worktree with inline message prompt |
| **Log** | `2` | Browse recent 300 commits on HEAD with metadata and colored patch diffs.<br>• `p` — Cherry-pick the selected commit onto current HEAD |
| **Branches** | `3` | Browse local branches and their tips (current HEAD marked with `*`).<br>• `Enter` — Checkout selected branch |
| **Stash** | `4` | Browse stashes and inspect their underlying patches.<br>• `Enter` or `a` — Apply stash<br>• `p` — Pop stash<br>• `d` — Drop stash |
| **Conflicts** | `5` | Resolve merge, rebase, or cherry-pick conflicts directly.<br>• `o` — Take **ours** (the current branch's version)<br>• `t` — Take **theirs** (the incoming branch's version)<br>• `m` — Mark conflict as resolved |

---

## ⌨️ Global Keybindings

| Key | Action |
| :--- | :--- |
| `1` – `5` | Switch directly to view (Status, Log, Branches, Stash, Conflicts) |
| `Tab` | Cycle forward through views |
| `j` / `Down` | Move down in list |
| `k` / `Up` | Move up in list |
| `PageDown` / `PageUp` | Scroll preview pane down/up by 10 lines |
| `r` | Refresh repository snapshot |
| `Esc` | Cancel an active text input prompt |
| `q` or `Ctrl+C` | Quit GitPilot and restore terminal |

---

## 🏗️ Architecture & Concurrency

- **Off-Thread Operations**: Because `git2::Repository` is not thread-safe (`!Sync`), all Git operations, status queries, and diff computations run in short-lived background worker threads.
- **Responsive Event Loop**: The main UI thread never blocks on disk I/O or repository locks. Updates flow via an `std::sync::mpsc` channel.
- **Race Condition Guard**: Previews are tagged with a unique selection key (`preview_key`) to discard stale diff computations when navigating rapidly across files.

---

## 🤝 Contributing

Contributions, bug reports, and suggestions are welcome!

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/cockpit-enhancement`)
3. Commit your changes (`git commit -am 'Add some cockpit feature'`)
4. Push to the branch (`git push origin feature/cockpit-enhancement`)
5. Open a Pull Request

---

## 📜 License

This project is licensed under the MIT License — see the [LICENSE](LICENSE) file for details.
