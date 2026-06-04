use crate::detect::Manager;
use crate::execute::{self, ManagerStatus, Stage};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal;
use std::collections::BTreeMap;
use std::io;
use tokio::sync::mpsc;

pub async fn run(managers: Vec<Manager>) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (tx, mut rx) = mpsc::unbounded_channel::<ManagerStatus>();

    let mut selected: usize = 0;
    let mut selected_map: BTreeMap<String, bool> = managers
        .iter()
        .map(|manager| (manager.key.clone(), true))
        .collect();
    let mut running = false;
    let mut runner: Option<tokio::task::JoinHandle<Result<()>>> = None;

    let mut status_map: BTreeMap<String, ManagerStatus> = BTreeMap::new();
    for manager in &managers {
        status_map.insert(
            manager.key.clone(),
            ManagerStatus {
                manager: manager.clone(),
                stage: Stage::Pending,
                message: None,
            },
        );
    }

    loop {
        while let Ok(update) = rx.try_recv() {
            status_map.insert(update.manager.key.clone(), update);
        }

        terminal.draw(|frame| {
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(80), Constraint::Percentage(20)])
                .split(frame.size());

            let items: Vec<ListItem> = status_map
                .values()
                .enumerate()
                .map(|(idx, status)| {
                    let checked = selected_map.get(&status.manager.key).copied().unwrap_or(false);
                    let label = format!(
                        "{} {} [{}]",
                        if checked { "[x]" } else { "[ ]" },
                        status.manager.config.name,
                        stage_label(status.stage)
                    );
                    let style = if idx == selected {
                        Style::default().fg(Color::Black).bg(Color::White)
                    } else {
                        Style::default()
                    };
                    ListItem::new(Line::from(Span::styled(label, style)))
                })
                .collect();

            let title = if running { "Spine" } else { "Spine (select, space to toggle, r to run)" };
            let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
            frame.render_widget(list, layout[0]);

            let detail = status_map.values().nth(selected).map(|status| {
                let mut lines = vec![Line::from(status.manager.config.name.as_str())];
                if let Some(message) = &status.message {
                    lines.push(Line::from(message.as_str()));
                }
                lines
            });
            let detail = detail.unwrap_or_else(|| vec![Line::from("No selection")]);
            let paragraph = Paragraph::new(detail)
                .block(Block::default().borders(Borders::ALL).title("Details"));
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
                            if selected > 0 {
                                selected -= 1;
                            }
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
                                    .filter(|manager| *selected_map.get(&manager.key).unwrap_or(&false))
                                    .cloned()
                                    .collect();
                                runner = Some(tokio::spawn(execute::run_with_updates(filtered, tx.clone())));
                                running = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if running {
            let all_done = status_map
                .values()
                .all(|status| matches!(status.stage, Stage::Complete | Stage::Failed));
            if all_done {
                break;
            }
        }
    }

    if let Some(handle) = runner {
        let _ = handle.await?;
    }
    disable_raw_mode()?;
    crossterm::execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Pending => "Pending",
        Stage::Refreshing => "Refreshing",
        Stage::Upgrading => "Upgrading",
        Stage::Cleaning => "Cleaning",
        Stage::Complete => "Complete",
        Stage::Failed => "Failed",
    }
}
