use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, List, ListItem, Paragraph, Tabs},
    Frame,
};

use crate::app::{App, Input, View};

/// Lemon green: the interface accent colour.
const ACCENT: Color = Color::Rgb(200, 255, 0);

pub fn draw(f: &mut Frame, app: &mut App) {
    let [tabs_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(2),
    ])
    .areas(f.area());

    let titles: Vec<Line> = View::ALL
        .iter()
        .enumerate()
        .map(|(i, v)| Line::from(format!(" {} {} ", i + 1, v.title())))
        .collect();
    let selected = View::ALL.iter().position(|v| *v == app.view).unwrap_or(0);
    let tabs = Tabs::new(titles)
        .select(selected)
        .block(
            Block::bordered()
                .border_style(Style::default().fg(ACCENT))
                .title(format!(" GitPilot · {} ", app.snap.branch)),
        )
        .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        .divider(" ");
    f.render_widget(tabs, tabs_area);

    let [list_area, preview_area] =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
            .areas(body_area);

    let items = list_items(app);
    let list = List::new(items)
        .block(
            Block::bordered()
                .border_style(Style::default().fg(ACCENT))
                .title(format!(" {} ", app.view.title())),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, list_area, &mut app.list);

    let preview = Paragraph::new(colorize(&app.preview))
        .block(
            Block::bordered()
                .border_style(Style::default().fg(ACCENT))
                .title(" preview "),
        )
        .scroll((app.preview_scroll, 0));
    f.render_widget(preview, preview_area);

    draw_footer(f, footer_area, app);
}

fn list_items(app: &App) -> Vec<ListItem<'static>> {
    match app.view {
        View::Status => app
            .snap
            .files
            .iter()
            .map(|f| {
                let (tag, color) = if f.staged {
                    ("staged", ACCENT)
                } else if f.code == '?' {
                    ("untracked", ACCENT)
                } else {
                    ("unstaged", Color::Red)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{} {:<9} ", f.code, tag),
                        Style::default().fg(color),
                    ),
                    Span::raw(f.path.clone()),
                ]))
            })
            .collect(),
        View::Log => app
            .snap
            .commits
            .iter()
            .map(|c| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", c.id), Style::default().fg(ACCENT)),
                    Span::styled(format!("{} ", c.date), Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{:<12.12} ", c.author),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(c.summary.clone()),
                ]))
            })
            .collect(),
        View::Branches => app
            .snap
            .branches
            .iter()
            .map(|b| {
                let (mark, style) = if b.is_head {
                    (
                        "* ",
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("  ", Style::default())
                };
                ListItem::new(Line::styled(format!("{mark}{}", b.name), style))
            })
            .collect(),
        View::Stash => app
            .snap
            .stashes
            .iter()
            .map(|s| ListItem::new(format!("stash@{{{}}}  {}", s.index, s.message)))
            .collect(),
        View::Conflicts => app
            .snap
            .conflicts
            .iter()
            .map(|p| ListItem::new(Line::styled(p.clone(), Style::default().fg(Color::Magenta))))
            .collect(),
    }
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let [msg_area, help_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);

    let msg = match &app.input {
        Some(Input::Commit(buf)) => Line::from(vec![
            Span::styled("commit message: ", Style::default().fg(ACCENT)),
            Span::raw(format!("{buf}▏")),
        ]),
        Some(Input::Stash(buf)) => Line::from(vec![
            Span::styled("stash message: ", Style::default().fg(ACCENT)),
            Span::raw(format!("{buf}▏")),
        ]),
        None => {
            let color = if app.notice_is_error {
                Color::Red
            } else {
                ACCENT
            };
            let spinner = if app.pending > 0 { "  ⟳" } else { "" };
            Line::styled(
                format!("{}{spinner}", app.notice),
                Style::default().fg(color),
            )
        }
    };
    f.render_widget(Paragraph::new(msg), msg_area);

    let help = if app.input.is_some() {
        "enter confirm · esc cancel".to_string()
    } else {
        format!(
            "1-5/tab views · j/k move · pgup/pgdn scroll · r refresh · q quit │ {}",
            view_help(app.view)
        )
    };
    f.render_widget(
        Paragraph::new(Line::styled(help, Style::default().fg(Color::DarkGray))),
        help_area,
    );
}

fn view_help(view: View) -> &'static str {
    match view {
        View::Status => "space stage/unstage · a stage all · c commit · S stash",
        View::Log => "p cherry-pick onto HEAD",
        View::Branches => "enter checkout",
        View::Stash => "enter/a apply · p pop · d drop",
        View::Conflicts => "o take ours · t take theirs · m mark resolved",
    }
}

/// Colours patch and conflict-marker lines. Caps output so huge diffs stay
/// cheap to render on every frame.
fn colorize(text: &str) -> Text<'static> {
    let lines: Vec<Line<'static>> = text
        .lines()
        .take(5000)
        .map(|l| {
            let style = if l.starts_with("+++") || l.starts_with("---") {
                Style::default().add_modifier(Modifier::BOLD)
            } else if l.starts_with('+') {
                Style::default().fg(ACCENT)
            } else if l.starts_with('-') {
                Style::default().fg(Color::Red)
            } else if l.starts_with("@@") {
                Style::default().fg(Color::Cyan)
            } else if l.starts_with("diff ") || l.starts_with("commit ") {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else if l.starts_with("<<<<<<<")
                || l.starts_with("=======")
                || l.starts_with(">>>>>>>")
            {
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::styled(l.to_string(), style)
        })
        .collect();
    Text::from(lines)
}
