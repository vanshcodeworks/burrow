//! `burrow top`: the interactive live dashboard.
//!
//! All terminal handling, key bindings, and control-socket traffic live here.
//! `burrow-tui` only turns a [`burrow_tui::Dashboard`] into cells, which keeps
//! the widgets testable without a real console.

use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::commands::Options;
use crate::control::{self, ControlCommand};
use crate::project::Project;
use crate::state::{unix_now, RuntimeState};
use burrow_tui::{Dashboard, LogView, ServiceView};

/// How many log lines the dashboard keeps.
const LOG_LIMIT: usize = 500;
/// How often state and logs are refreshed.
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// Implements `burrow top`.
pub async fn run(options: &Options) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!(
            "`burrow top` needs an interactive terminal\n\
             help: use `burrow status` or `burrow logs -f` in scripts and CI"
        );
    }

    let project = Project::load(options.project.clone())?;
    let mut dashboard = Dashboard::new(project.config.project.name.clone());
    let _session = TerminalSession::start()?;
    let stdout = std::io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal =
        Terminal::new(backend).context("could not initialize the terminal dashboard")?;
    terminal.clear().context("could not clear the terminal")?;

    let mut state: Option<RuntimeState> = None;
    let mut last_seq = 0u64;
    let mut daemon_pid: Option<u32> = None;

    loop {
        refresh(
            &project,
            &mut dashboard,
            &mut state,
            &mut last_seq,
            &mut daemon_pid,
        )
        .await;
        dashboard.clamp_selection();
        terminal
            .draw(|frame| dashboard.render(frame))
            .context("could not draw the dashboard")?;

        let deadline = Instant::now() + REFRESH_INTERVAL;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let wait = remaining.min(Duration::from_millis(100));
            if !event::poll(wait).context("could not poll the terminal")? {
                continue;
            }
            let Event::Key(key) = event::read().context("could not read a terminal event")? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(())
                }
                KeyCode::Up | KeyCode::Char('k') => dashboard.move_selection(-1),
                KeyCode::Down | KeyCode::Char('j') => dashboard.move_selection(1),
                KeyCode::Char('r') => {
                    restart_selected(&mut terminal, &mut dashboard, &state).await?;
                    refresh(
                        &project,
                        &mut dashboard,
                        &mut state,
                        &mut last_seq,
                        &mut daemon_pid,
                    )
                    .await;
                }
                _ => {}
            }
        }
    }
}

async fn refresh(
    project: &Project,
    dashboard: &mut Dashboard,
    state: &mut Option<RuntimeState>,
    last_seq: &mut u64,
    daemon_pid: &mut Option<u32>,
) {
    dashboard.daemon_running = false;
    let stored = match RuntimeState::read(&project.dir) {
        Ok(Some(state)) => state,
        Ok(None) => {
            dashboard.services.clear();
            dashboard.ready = false;
            dashboard.proxy_port = None;
            dashboard.message = "Burrow is not running; press q to quit, or run `burrow up`".into();
            return;
        }
        Err(error) => {
            dashboard.message = format!("could not read state: {error:#}");
            return;
        }
    };

    // Keep the last known service list visible while the daemon is down.
    apply_state(dashboard, &stored);
    match control::request(
        &stored,
        ControlCommand::Logs {
            service: None,
            since_seq: *last_seq,
            limit: LOG_LIMIT,
        },
    )
    .await
    {
        Ok(response) if response.ok => {
            dashboard.daemon_running = true;
            dashboard.message.clear();
            if let Some(live) = &response.state {
                if *daemon_pid != Some(live.pid) {
                    // A new daemon means a new log sequence.
                    *daemon_pid = Some(live.pid);
                    *last_seq = 0;
                    dashboard.logs.clear();
                }
                apply_state(dashboard, live);
                *state = response.state.clone();
            }
            for entry in response.logs {
                *last_seq = (*last_seq).max(entry.seq);
                dashboard.logs.push(LogView {
                    service: entry.service,
                    stream: entry.stream,
                    text: entry.text,
                });
            }
            if dashboard.logs.len() > LOG_LIMIT {
                let excess = dashboard.logs.len() - LOG_LIMIT;
                dashboard.logs.drain(..excess);
            }
        }
        Ok(response) => {
            dashboard.message = response.message;
        }
        Err(error) => {
            dashboard.message = format!("daemon unreachable: {error}");
        }
    }
}

fn apply_state(dashboard: &mut Dashboard, state: &RuntimeState) {
    dashboard.project = state.project.clone();
    dashboard.ready = state.ready;
    dashboard.proxy_port = state.proxy_port;
    let now = unix_now();
    dashboard.services = state
        .services
        .iter()
        .map(|(name, snapshot)| ServiceView {
            name: name.clone(),
            status: snapshot.status.label().to_owned(),
            port: snapshot.port,
            cpu_percent: snapshot.cpu_percent,
            memory_bytes: snapshot.memory_bytes,
            restarts: snapshot.restarts,
            uptime_secs: snapshot
                .pid
                .and(snapshot.started_at)
                .map(|started| now.saturating_sub(started))
                .unwrap_or(0),
            url: snapshot.url.clone(),
            route_url: snapshot.route_url.clone(),
        })
        .collect();
}

async fn restart_selected<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    dashboard: &mut Dashboard,
    state: &Option<RuntimeState>,
) -> Result<()> {
    let Some(service) = dashboard
        .selected_service()
        .map(|service| service.name.clone())
    else {
        dashboard.message = "there is no service to restart".to_owned();
        return Ok(());
    };
    let Some(state) = state else {
        dashboard.message = "the daemon is not running".to_owned();
        return Ok(());
    };

    dashboard.message = format!("restarting {service}...");
    terminal
        .draw(|frame| dashboard.render(frame))
        .context("could not draw the dashboard")?;
    match control::request_with_timeout(
        state,
        ControlCommand::Restart {
            service: service.clone(),
        },
        Duration::from_secs(120),
    )
    .await
    {
        Ok(response) if response.ok => {
            dashboard.message = if response.message.is_empty() {
                format!("{service} restarted")
            } else {
                response.message
            };
        }
        Ok(response) => dashboard.message = response.message,
        Err(error) => dashboard.message = format!("could not restart {service}: {error}"),
    }
    Ok(())
}

/// Owns the terminal modes and restores them on every exit path, including
/// errors and panics.
struct TerminalSession {
    active: bool,
}

impl TerminalSession {
    fn start() -> Result<Self> {
        enable_raw_mode().context("could not enable raw terminal mode")?;
        let mut session = Self { active: true };
        execute!(std::io::stdout(), EnterAlternateScreen)
            .context("could not enter the alternate screen")?;
        session.active = true;
        Ok(session)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let _ = disable_raw_mode();
        let mut stdout = std::io::stdout();
        let _ = execute!(
            stdout,
            LeaveAlternateScreen,
            crossterm::cursor::Show,
            crossterm::style::ResetColor
        );
        let _ = stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::apply_state;
    use crate::state::{RuntimeState, ServiceSnapshot, ServiceStatus, STATE_VERSION};
    use burrow_tui::Dashboard;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn state() -> RuntimeState {
        RuntimeState {
            version: STATE_VERSION,
            project: "shop".to_owned(),
            project_dir: PathBuf::from(r"C:\shop"),
            pid: 42,
            token: "secret".to_owned(),
            control_port: 43000,
            proxy_port: Some(80),
            started_at: 1,
            config_hash: "hash".to_owned(),
            ready: true,
            services: BTreeMap::from([(
                "web".to_owned(),
                ServiceSnapshot {
                    status: ServiceStatus::Healthy,
                    pid: Some(43),
                    port: 43123,
                    url: "http://127.0.0.1:43123".to_owned(),
                    route: Some("shop.localhost".to_owned()),
                    route_url: Some("http://shop.localhost".to_owned()),
                    restarts: 2,
                    started_at: Some(super::unix_now().saturating_sub(65)),
                    exit_code: None,
                    memory_bytes: 48 * 1024 * 1024,
                    cpu_percent: 1.5,
                    last_error: None,
                },
            )]),
        }
    }

    #[test]
    fn state_maps_to_dashboard_rows() {
        let mut dashboard = Dashboard::new("before");
        apply_state(&mut dashboard, &state());
        assert_eq!(dashboard.project, "shop");
        assert!(dashboard.ready);
        assert_eq!(dashboard.proxy_port, Some(80));
        let service = dashboard.selected_service().expect("service");
        assert_eq!(service.name, "web");
        assert_eq!(service.status, "healthy");
        assert_eq!(service.endpoint(), "http://shop.localhost");
        assert_eq!(service.restarts, 2);
        assert_eq!(service.uptime_secs, 65);
    }
}
