//! `<agente> top`: TUI mínima que refresca el status socket cada segundo.
//! Deliberadamente genérica — no sabe nada de métricas, hallazgos de
//! seguridad ni fases de deploy. Renderiza los campos comunes de
//! `StatusPayload` y el `details` libre como JSON bonito. Cada agente
//! obtiene esto gratis con una sola llamada; el "widget" específico por
//! agente (si algún día hace falta) sería trabajo aparte, no de este crate.

use crate::status_client;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, ExecutableCommand};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use std::io::stdout;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Corre la TUI hasta que el usuario pulsa `q`/Esc/Ctrl+C. Bloqueante.
pub fn run_top(agent_name: &str) -> std::io::Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, agent_name);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    agent_name: &str,
) -> std::io::Result<()> {
    let mut last_error: Option<String> = None;

    loop {
        let snapshot = status_client::read_once_timeout(agent_name, Duration::from_millis(500));
        if let Err(e) = &snapshot {
            last_error = Some(e.to_string());
        }

        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::vertical([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(area);

            let title = Paragraph::new(Line::from(vec![Span::styled(
                format!(" {agent_name} — sb-agent-core top "),
                Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
            )]))
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(title, chunks[0]);

            match &snapshot {
                Ok(payload) => {
                    let state_color = match payload.state.as_str() {
                        "running" => Color::Green,
                        "starting" => Color::Yellow,
                        "stopping" => Color::Red,
                        _ => Color::White,
                    };
                    let uptime_secs = now_unix().saturating_sub(payload.since_unix);
                    let summary = Paragraph::new(Line::from(vec![
                        Span::raw("state: "),
                        Span::styled(payload.state.clone(), Style::default().fg(state_color).add_modifier(Modifier::BOLD)),
                        Span::raw(format!("   version: {}   uptime: {}s", payload.version, uptime_secs)),
                    ]))
                    .block(Block::default().borders(Borders::ALL).title(" summary "));
                    frame.render_widget(summary, chunks[1]);

                    let details_text = serde_json::to_string_pretty(&payload.details)
                        .unwrap_or_else(|_| "null".to_string());
                    let details = Paragraph::new(details_text)
                        .block(Block::default().borders(Borders::ALL).title(" details "));
                    frame.render_widget(details, chunks[2]);
                }
                Err(_) => {
                    let msg = last_error.clone().unwrap_or_else(|| "no data yet".to_string());
                    let err_widget = Paragraph::new(Line::from(vec![Span::styled(
                        format!("could not read status socket: {msg}"),
                        Style::default().fg(Color::Red),
                    )]))
                    .block(Block::default().borders(Borders::ALL).title(" summary "));
                    frame.render_widget(err_widget, chunks[1]);
                }
            }

            let footer = Paragraph::new(" q / Esc: quit — refreshes every 1s ");
            frame.render_widget(footer, chunks[3]);
        })?;

        if event::poll(Duration::from_millis(1000))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press
                    && (key.code == KeyCode::Char('q') || key.code == KeyCode::Esc)
                {
                    return Ok(());
                }
            }
        }
    }
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
