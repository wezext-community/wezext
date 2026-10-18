// ============================================================
//  wezext interactive UI (ratatui)
//
//  Two pages (Tab to switch):
//    Extensions - indexed extensions and their install state; install / update /
//                 remove / build from source right here
//    Doctor     - output of the doctor checks (files present, config wired up)
//
//  Keys: up/down/jk select - Enter install or update - b build from source
//        d remove - r refresh index - u self-update - Tab switch page - q/Esc quit
// ============================================================
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::{
    do_install, doctor_lines, load_index, load_state, regenerate_lua, save_state, InstallMode,
    Installed, SELF_VERSION,
};

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Extensions,
    Doctor,
}

/// Result of an action, shown for a few seconds
struct Flash {
    text: String,
    ok: bool,
    at: Instant,
}

struct App {
    index: crate::Index,
    index_src: String,
    state: crate::State,
    sel: usize,
    page: Page,
    doctor: Vec<String>,
    doctor_problems: usize,
    flash: Option<Flash>,
    busy: Option<String>,
    quit: bool,
}

impl App {
    fn new() -> Self {
        let (index, index_src) = match load_index(false) {
            Ok(v) => v,
            Err(e) => {
                // still show the UI when the index fails, so the error is visible
                eprintln!("failed to load index: {e}");
                (
                    crate::Index {
                        schema: 1,
                        extensions: Default::default(),
                    },
                    format!("load failed: {e}"),
                )
            }
        };
        let state = load_state();
        let (doctor, doctor_problems) = doctor_lines(&index, &state);
        App {
            index,
            index_src,
            state,
            sel: 0,
            page: Page::Extensions,
            doctor,
            doctor_problems,
            flash: None,
            busy: None,
            quit: false,
        }
    }

    fn names(&self) -> Vec<String> {
        self.index.extensions.keys().cloned().collect()
    }

    fn selected(&self) -> Option<(&String, &crate::Extension)> {
        let name = self.names().into_iter().nth(self.sel)?;
        let ext = self.index.extensions.get(&name)?;
        Some((self.index.extensions.get_key_value(&name)?.0, ext))
    }

    fn refresh_doctor(&mut self) {
        let (lines, problems) = doctor_lines(&self.index, &self.state);
        self.doctor = lines;
        self.doctor_problems = problems;
    }

    fn flash_ok(&mut self, text: String) {
        self.flash = Some(Flash {
            text,
            ok: true,
            at: Instant::now(),
        });
    }

    fn flash_err(&mut self, text: String) {
        self.flash = Some(Flash {
            text,
            ok: false,
            at: Instant::now(),
        });
    }

    /// Install or update the selection (mode is picked automatically)
    fn install_selected(&mut self, force_build: bool) {
        let Some((name, _)) = self.selected().map(|(n, e)| (n.clone(), e.version.clone())) else {
            return;
        };
        let mode = if force_build {
            Some(InstallMode::Source)
        } else {
            None
        };
        self.busy = Some(format!(
            "{} {name}...",
            if force_build { "building" } else { "installing" }
        ));
        let result = do_install(&name, mode, &self.index, &mut self.state);
        self.busy = None;
        match result {
            Ok(()) => {
                let _ = save_state(&self.state);
                let _ = regenerate_lua(&self.state, &self.index);
                let done = self
                    .state
                    .extensions
                    .get(&name)
                    .map(|i| i.version.clone())
                    .unwrap_or_default();
                self.flash_ok(format!("installed {name} {done} (regenerated .wezterm-ext.lua)"));
                self.refresh_doctor();
            }
            Err(e) => self.flash_err(format!("install failed: {e}")),
        }
    }

    fn remove_selected(&mut self) {
        let Some((name, _)) = self.selected().map(|(n, e)| (n.clone(), e.version.clone())) else {
            return;
        };
        let Some(inst) = self.state.extensions.remove(&name) else {
            self.flash_err(format!("{name} is not installed"));
            return;
        };
        let p = PathBuf::from(&inst.path);
        if inst.kind == "lua" {
            let _ = std::fs::remove_file(&p);
        } else if p.exists() {
            let _ = std::fs::remove_dir_all(&p);
        }
        let _ = save_state(&self.state);
        let _ = regenerate_lua(&self.state, &self.index);
        self.flash_ok(format!("removed {name} {}", inst.version));
        self.refresh_doctor();
    }

    fn refresh_index(&mut self) {
        self.busy = Some("refreshing index...".into());
        match load_index(true) {
            Ok((index, src)) => {
                self.index = index;
                self.index_src = src;
                self.flash_ok(format!("index refreshed: {} extensions", self.index.extensions.len()));
                if self.sel >= self.index.extensions.len() {
                    self.sel = self.index.extensions.len().saturating_sub(1);
                }
                self.state = load_state();
                self.refresh_doctor();
            }
            Err(e) => self.flash_err(format!("index refresh failed: {e}")),
        }
        self.busy = None;
    }

    fn self_update(&mut self) {
        self.busy = Some("updating wezext...".into());
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("wezext.exe"));
        let out = Command::new(exe).arg("self-update").output();
        self.busy = None;
        match out {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
                self.flash_ok(text.lines().last().unwrap_or("self-update finished").to_string());
            }
            Ok(o) => {
                let text = String::from_utf8_lossy(&o.stderr).trim().to_string();
                self.flash_err(format!("self-update failed: {text}"));
            }
            Err(e) => self.flash_err(format!("cannot run self-update: {e}")),
        }
    }

    fn installed_of(&self, name: &str) -> Option<&Installed> {
        self.state.extensions.get(name)
    }
}

pub fn run() -> i32 {
    let mut terminal = ratatui::init();
    let mut app = App::new();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("TUI error: {e}");
            1
        }
    }
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> std::io::Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if let Some(flash) = &app.flash {
            if flash.at.elapsed() > Duration::from_secs(6) {
                app.flash = None;
            }
        }

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
            KeyCode::Tab => {
                app.page = if app.page == Page::Extensions {
                    Page::Doctor
                } else {
                    Page::Extensions
                };
                app.refresh_doctor();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.sel + 1 < app.index.extensions.len() {
                    app.sel += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.sel = app.sel.saturating_sub(1);
            }
            KeyCode::Enter => app.install_selected(false),
            KeyCode::Char('b') => app.install_selected(true),
            KeyCode::Char('d') => app.remove_selected(),
            KeyCode::Char('r') => app.refresh_index(),
            KeyCode::Char('u') => app.self_update(),
            _ => {}
        }

        if app.quit {
            break;
        }
    }
    Ok(())
}

fn status_badge(app: &App, name: &str) -> Span<'static> {
    match (app.installed_of(name), app.index.extensions.get(name)) {
        (Some(inst), Some(ext)) if inst.version == ext.version => {
            Span::styled(" installed ", Style::default().fg(Color::Black).bg(Color::LightGreen))
        }
        (Some(_), Some(_)) => {
            Span::styled(" update ", Style::default().fg(Color::Black).bg(Color::Yellow))
        }
        (Some(_), None) => Span::styled(" not in index ", Style::default().fg(Color::Black).bg(Color::Red)),
        _ => Span::styled(" not installed ", Style::default().fg(Color::Black).bg(Color::DarkGray)),
    }
}

fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(f.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(chunks[1]);

    // header
    let page_badge = match app.page {
        Page::Extensions => Span::styled(
            " Extensions ",
            Style::default().fg(Color::Black).bg(Color::LightCyan),
        ),
        Page::Doctor => Span::styled(
            " Doctor ",
            Style::default().fg(Color::Black).bg(Color::LightYellow),
        ),
    };
    let mut header = vec![
        Span::styled(
            format!(" wezext {SELF_VERSION} "),
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        page_badge,
        Span::raw(format!(
            "   {} extensions, {} installed",
            app.index.extensions.len(),
            app.state.extensions.len()
        )),
    ];
    if let Some(b) = &app.busy {
        header.push(Span::styled(
            format!("   ⏳ {b}"),
            Style::default().fg(Color::Yellow),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(header)), chunks[0]);

    match app.page {
        Page::Extensions => {
            // left: list
            let mut items: Vec<ListItem> = Vec::new();
            for (name, ext) in &app.index.extensions {
                let inst = app.installed_of(name);
                let ver = inst.map(|i| i.version.clone()).unwrap_or_default();
                let mut spans = vec![
                    Span::raw(format!(" {name} ")),
                    status_badge(app, name),
                    Span::styled(
                        format!("  {}", if ver.is_empty() { ext.version.clone() } else { ver }),
                        Style::default().fg(Color::Gray),
                    ),
                ];
                if let Some(i) = inst {
                    spans.push(Span::styled(
                        format!("  [{}]", i.kind),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                items.push(ListItem::new(Line::from(spans)));
            }
            if items.is_empty() {
                items.push(ListItem::new(Line::from(Span::styled(
                    " (index is empty - press r to refresh)",
                    Style::default().fg(Color::DarkGray),
                ))));
            }
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(" Extensions "))
                .highlight_style(
                    Style::default()
                        .bg(Color::LightCyan)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ");
            let mut st = ListState::default();
            if !app.index.extensions.is_empty() {
                st.select(Some(app.sel));
            }
            f.render_stateful_widget(list, body[0], &mut st);

            // right: details
            let mut lines: Vec<Line> = Vec::new();
            if let Some((name, ext)) = app.selected() {
                lines.push(Line::from(Span::styled(
                    ext.display.clone(),
                    Style::default()
                        .fg(Color::LightYellow)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    name.clone(),
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(""));
                for chunk in textwrap_lines(&ext.description, body[1].width.saturating_sub(4) as usize)
                {
                    lines.push(Line::from(chunk));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("index version ", Style::default().fg(Color::DarkGray)),
                    Span::raw(ext.version.clone()),
                ]));
                if let Some(i) = app.installed_of(name) {
                    lines.push(Line::from(vec![
                        Span::styled("installed ", Style::default().fg(Color::DarkGray)),
                        Span::raw(format!("{} ({})", i.version, i.kind)),
                    ]));
                    lines.push(Line::from(Span::styled(
                        i.exe.clone(),
                        Style::default().fg(Color::Gray),
                    )));
                }
                let mut kinds = Vec::new();
                if ext.binary.is_some() {
                    kinds.push("binary");
                }
                if ext.source.is_some() {
                    kinds.push("source");
                }
                if ext.lua.is_some() {
                    kinds.push("lua snippet");
                }
                lines.push(Line::from(vec![
                    Span::styled("available as ", Style::default().fg(Color::DarkGray)),
                    Span::raw(kinds.join(" / ")),
                ]));
                if !ext.homepage.is_empty() {
                    lines.push(Line::from(Span::styled(
                        ext.homepage.clone(),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                if !ext.keys.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "key bindings (active once installed)",
                        Style::default().fg(Color::DarkGray),
                    )));
                    for k in &ext.keys {
                        lines.push(Line::from(format!(
                            "  {} + {}  {}",
                            k.mods,
                            k.key,
                            k.hint.clone()
                        )));
                    }
                }
            } else {
                lines.push(Line::from("(nothing selected)"));
            }
            f.render_widget(
                Paragraph::new(lines)
                    .block(Block::default().borders(Borders::ALL).title(" Details "))
                    .wrap(Wrap { trim: false }),
                body[1],
            );
        }
        Page::Doctor => {
            let mut lines: Vec<Line> = Vec::new();
            for l in &app.doctor {
                let style = if l.starts_with("[ok]") {
                    Style::default().fg(Color::Green)
                } else if l.starts_with("[!!]") {
                    Style::default().fg(Color::LightRed)
                } else {
                    Style::default().fg(Color::Gray)
                };
                lines.push(Line::from(Span::styled(l.clone(), style)));
            }
            let title = if app.doctor_problems == 0 {
                " Doctor: all good ".to_string()
            } else {
                format!(" Doctor: {} problem(s) ", app.doctor_problems)
            };
            f.render_widget(
                Paragraph::new(lines)
                    .block(Block::default().borders(Borders::ALL).title(title))
                    .wrap(Wrap { trim: false }),
                body[1].union(body[0]),
            );
        }
    }

    // footer
    let help = Line::from(vec![
        Span::styled(" Tab ", Style::default().fg(Color::LightCyan)),
        Span::raw("page  "),
        Span::styled("↑↓/jk ", Style::default().fg(Color::LightBlue)),
        Span::raw("select  "),
        Span::styled("Enter ", Style::default().fg(Color::LightBlue)),
        Span::raw("install/update  "),
        Span::styled("b ", Style::default().fg(Color::LightBlue)),
        Span::raw("build from source  "),
        Span::styled("d ", Style::default().fg(Color::LightBlue)),
        Span::raw("remove  "),
        Span::styled("r ", Style::default().fg(Color::LightBlue)),
        Span::raw("refresh index  "),
        Span::styled("u ", Style::default().fg(Color::LightBlue)),
        Span::raw("self-update  "),
        Span::styled("q ", Style::default().fg(Color::LightBlue)),
        Span::raw("quit"),
    ]);
    f.render_widget(Paragraph::new(help), chunks[2]);

    // second footer line: last result, or the index source
    let status_area = ratatui::layout::Rect {
        x: chunks[2].x,
        y: chunks[2].y + 1,
        width: chunks[2].width,
        height: 1,
    };
    let text = if let Some(flash) = &app.flash {
        let style = if flash.ok {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::LightRed)
        };
        Line::from(Span::styled(format!(" {}", flash.text), style))
    } else {
        Line::from(Span::styled(
            format!(" index: {}", app.index_src),
            Style::default().fg(Color::DarkGray),
        ))
    };
    f.render_widget(Paragraph::new(text), status_area);
}

/// Naive width-based wrapping (by chars, wide CJK counts as 2)
fn textwrap_lines(text: &str, width: usize) -> Vec<String> {
    let width = width.max(10);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut n = 0usize;
    for ch in text.chars() {
        if n >= width {
            out.push(std::mem::take(&mut cur));
            n = 0;
        }
        cur.push(ch);
        n += if (ch as u32) > 0x2E80 { 2 } else { 1 };
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
