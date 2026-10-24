mod app;
mod git;
mod ui;

use std::{path::PathBuf, sync::mpsc, time::Duration};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyEventKind};
use git2::Repository;
use ratatui::DefaultTerminal;

use app::{App, Msg};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "-v" | "--version" | "-V" => {
                println!("gitpilot {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-h" | "--help" => {
                println!(
                    "GitPilot {}\n{}\n\nUsage: gitpilot [OPTIONS]\n\nOptions:\n  -h, --help     Print help information\n  -V, --version  Print version",
                    env!("CARGO_PKG_VERSION"),
                    env!("CARGO_PKG_DESCRIPTION")
                );
                return Ok(());
            }
            _ => {}
        }
    }

    let workdir: PathBuf = {
        let repo = Repository::discover(".")
            .context("GitPilot must be started inside a Git repository")?;
        let path = repo
            .workdir()
            .context("bare repositories are not supported")?
            .to_path_buf();
        path
    };

    // ratatui::init enables raw mode, switches to the alternate screen and
    // installs a panic hook that restores the terminal first.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, workdir);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, workdir: PathBuf) -> Result<()> {
    let (tx, rx) = mpsc::channel::<Msg>();
    let mut app = App::new(workdir, tx);
    app.refresh();

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        // Apply finished background work before handling the next key.
        while let Ok(msg) = rx.try_recv() {
            app.on_msg(msg);
        }

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.on_key(key);
                }
            }
        }

        if app.quit {
            break;
        }
    }
    Ok(())
}
