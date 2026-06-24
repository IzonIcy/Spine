use crate::detect::Manager;
use crate::execute::{self, ManagerEvent, ManagerStatus, RunOptions, Stage};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;
use std::collections::BTreeMap;
use std::io;
use std::time::Instant;
use tokio::sync::mpsc;

pub async fn run(managers: Vec<Manager>, options: RunOptions) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_inner(&mut terminal, managers, options).await;

    disable_raw_mode()?;
    crossterm::execute!(io::stdout(), LeaveAlternateScreen)?;
    result
}

async fn run_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    managers: Vec<Manager>,
    options: RunOptions,
) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<ManagerEvent>();

    let mut selected: usize = 0;
    let mut selected_map: BTreeMap<String, bool> = managers
        .iter()
        .map(|manager| (manager.key.clone(), true))
        .collect();
    let mut running = false;
    let mut runner: Option<tokio::task::JoinHandle<Result<execute::RunSummary>>> = None;

    let mut status_map: BTreeMap<String, ManagerStatus> = BTreeMap::new();
    for manager in &managers {
        status_map.insert(manager.key.clone(), ManagerStatus::pending(manager.clone()));
    }

    loop {
        while let Ok(event) = rx.try_recv() {
            match event {
                ManagerEvent::Status(status) => {
                    let key = status.manager.key.clone();
                    if let Some(existing) = status_map.get_mut(&key) {
                        let output = std::mem::take(&mut existing.output);
                        *existing = status;
                        existing.output = output;
                    } else {
                        status_map.insert(key, status);
                    }
                }
                ManagerEvent::Output { key, line } => {
                    if let Some(status) = status_map.get_mut(&key) {
                        status.output.push(line);
                        if status.output.len() > 500 {
                            let drain_count = status.output.len() - 500;
                            status.output.drain(0..drain_count);
                        }
                    }
                }
            }
        }

        terminal.draw(|frame| {
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
                .split(frame.size());

            let items: Vec<ListItem> = status_map
                .values()
                .enumerate()
                .map(|(idx, status)| {
                    let checked = selected_map
                        .get(&status.manager.key)
                        .copied()
                        .unwrap_or(false);
                    let label = format!(
                        "{} {} [{}] {}",
                        if checked { "[x]" } else { "[ ]" },
                        status.manager.config.name,
                        execute::stage_label(status.stage),
                        duration_label(status)
                    );
                    let style = if idx == selected {
                        Style::default().fg(Color::Black).bg(Color::White)
                    } else {
                        stage_style(status.stage)
                    };
                    ListItem::new(Line::from(Span::styled(label, style)))
                })
                .collect();

            let title = if running {
                format!("Spine ({})", options.workflow.label())
            } else {
                format!(
                    "Spine ({}, select, space to toggle, r to run, q to quit)",
                    options.workflow.label()
                )
            };
            let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
            frame.render_widget(list, layout[0]);

            let detail = status_map.values().nth(selected).map(|status| {
                let mut lines = vec![Line::from(format!(
                    "{} ({}) - {} {}",
                    status.manager.config.name,
                    status.manager.key,
                    execute::stage_label(status.stage),
                    duration_label(status)
                ))];
                if let Some(message) = &status.message {
                    lines.push(Line::from(message.as_str()));
                }
                if !status.output.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from("Output:"));
                    let output_start = status.output.len().saturating_sub(12);
                    for line in &status.output[output_start..] {
                        lines.push(Line::from(line.as_str()));
                    }
                }
                lines
            });
            let detail = detail.unwrap_or_else(|| vec![Line::from("No selection")]);
            let paragraph = Paragraph::new(detail)
                .block(Block::default().borders(Borders::ALL).title("Details"))
                .wrap(Wrap { trim: false });
            frame.render_widget(paragraph, layout[1]);
        })?;

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Down | KeyCode::Char('j') => {
                            if selected + 1 < status_map.len() {
                                selected += 1;
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            selected = selected.saturating_sub(1);
                        }
                        KeyCode::Char(' ') => {
                            if !running {
                                if let Some(item) = status_map.values().nth(selected) {
                                    let key = item.manager.key.clone();
                                    let entry = selected_map.entry(key).or_insert(true);
                                    *entry = !*entry;
                                }
                            }
                        }
                        KeyCode::Char('r') => {
                            if !running {
                                let filtered: Vec<Manager> = managers
                                    .iter()
                                    .filter(|manager| {
                                        *selected_map.get(&manager.key).unwrap_or(&false)
                                    })
                                    .cloned()
                                    .collect();
                                runner = Some(tokio::spawn(execute::run_with_updates(
                                    filtered,
                                    options,
                                    tx.clone(),
                                )));
                                running = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if running {
            let all_done = status_map.values().all(|status| {
                matches!(
                    status.stage,
                    Stage::Complete | Stage::Failed | Stage::Skipped
                )
            });
            if all_done {
                break;
            }
        }
    }

    if let Some(handle) = runner {
        handle.await??;
    }

    Ok(())
}

fn duration_label(status: &ManagerStatus) -> String {
    let Some(started_at) = status.started_at else {
        return "".to_string();
    };
    let elapsed = status
        .finished_at
        .unwrap_or_else(Instant::now)
        .duration_since(started_at);
    let seconds = elapsed.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn stage_style(stage: Stage) -> Style {
    match stage {
        Stage::Complete => Style::default().fg(Color::Green),
        Stage::Failed => Style::default().fg(Color::Red),
        Stage::Skipped => Style::default().fg(Color::Yellow),
        Stage::Checking | Stage::Refreshing | Stage::Upgrading | Stage::Cleaning => {
            Style::default().fg(Color::Cyan)
        }
        Stage::Pending => Style::default(),
    }
}
