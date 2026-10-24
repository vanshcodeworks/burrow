//! Terminal dashboard widgets for `burrow top`.
//!
//! The crate is deliberately free of networking and process code. The CLI
//! fetches a snapshot from the daemon, converts it into [`Dashboard`], and asks
//! this crate to render it. That separation keeps the layout unit-testable with
//! ratatui's `TestBackend` and avoids a dependency cycle with `burrow-cli`.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::Frame;

/// One service row in the dashboard.
#[derive(Debug, Clone)]
pub struct ServiceView {
    /// Service name.
    pub name: String,
    /// State label such as `healthy` or `crashed`.
    pub status: String,
    /// Allocated port.
    pub port: u16,
    /// CPU usage in percent.
    pub cpu_percent: f32,
    /// Working-set memory in bytes.
    pub memory_bytes: u64,
    /// Automatic restarts since the daemon started.
    pub restarts: u32,
    /// Seconds the current process has been running.
    pub uptime_secs: u64,
    /// Direct loopback endpoint.
    pub url: String,
    /// Browser URL through the reverse proxy, when routed.
    pub route_url: Option<String>,
}

impl ServiceView {
    /// The endpoint a user is most likely to want.
    pub fn endpoint(&self) -> &str {
        self.route_url.as_deref().unwrap_or(&self.url)
    }
}

/// One captured log line.
#[derive(Debug, Clone)]
pub struct LogView {
    /// Service that produced the line.
    pub service: String,
    /// `stdout` or `stderr`.
    pub stream: String,
    /// Line text.
    pub text: String,
}

/// Everything the dashboard needs for one frame.
#[derive(Debug, Clone)]
pub struct Dashboard {
    /// Project name.
    pub project: String,
    /// Whether the daemon answered the last status request.
    pub daemon_running: bool,
    /// Whether startup finished.
    pub ready: bool,
    /// Reverse-proxy port, when the project has routes.
    pub proxy_port: Option<u16>,
    /// Services, ordered by name.
    pub services: Vec<ServiceView>,
    /// Recent log lines, oldest first.
    pub logs: Vec<LogView>,
    /// Selected service index.
    pub selected: usize,
    /// Transient status or error shown in the footer.
    pub message: String,
}

impl Dashboard {
    /// Creates an empty dashboard for `project`.
    pub fn new(project: impl Into<String>) -> Self {
        Self {
            project: project.into(),
            daemon_running: false,
            ready: false,
            proxy_port: None,
            services: Vec::new(),
            logs: Vec::new(),
            selected: 0,
            message: String::new(),
        }
    }

    /// Keeps the selection inside the service list after a refresh.
    pub fn clamp_selection(&mut self) {
        if self.services.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.services.len() {
            self.selected = self.services.len() - 1;
        }
    }

    /// Moves the selection by `delta` rows, saturating at both ends.
    pub fn move_selection(&mut self, delta: isize) {
        if self.services.is_empty() {
            self.selected = 0;
            return;
        }
        let last = self.services.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    /// The selected service, if any.
    pub fn selected_service(&self) -> Option<&ServiceView> {
        self.services.get(self.selected)
    }

    /// Renders the dashboard into `frame`.
    pub fn render(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(10),
            Constraint::Length(1),
        ])
        .split(area);

        self.render_header(frame, chunks[0]);
        self.render_services(frame, chunks[1]);
        self.render_logs(frame, chunks[2]);
        self.render_footer(frame, chunks[3]);
    }

    fn render_header(&self, frame: &mut Frame<'_>, area: ratatui::layout::Rect) {
        let state = if !self.daemon_running {
            Span::styled("daemon unreachable", Style::default().fg(Color::Red))
        } else if self.ready {
            Span::styled("ready", Style::default().fg(Color::Green))
        } else {
            Span::styled("starting", Style::default().fg(Color::Yellow))
        };
        let mut spans = vec![
            Span::styled(
                format!(" Burrow · {} ", self.project),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            state,
        ];
        if let Some(port) = self.proxy_port {
            spans.push(Span::raw(format!("   proxy :{port}")));
        }
        let block = Block::default().borders(Borders::ALL).title(" burrow top ");
        frame.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
    }

    fn render_services(&self, frame: &mut Frame<'_>, area: ratatui::layout::Rect) {
        let header = Row::new([
            Cell::from("SERVICE"),
            Cell::from("STATE"),
            Cell::from("PORT"),
            Cell::from("CPU"),
            Cell::from("MEM"),
            Cell::from("RESTARTS"),
            Cell::from("UPTIME"),
            Cell::from("ENDPOINT"),
        ])
        .style(Style::default().add_modifier(Modifier::BOLD));

        let rows = self.services.iter().map(|service| {
            Row::new([
                Cell::from(service.name.clone()),
                Cell::from(service.status.clone())
                    .style(Style::default().fg(status_color(&service.status))),
                Cell::from(service.port.to_string()),
                Cell::from(format!("{:.1}%", service.cpu_percent)),
                Cell::from(format_memory(service.memory_bytes)),
                Cell::from(service.restarts.to_string()),
                Cell::from(format_uptime(service.uptime_secs)),
                Cell::from(service.endpoint().to_owned()),
            ])
        });

        let table = Table::new(
            rows,
            [
                Constraint::Length(16),
                Constraint::Length(10),
                Constraint::Length(6),
                Constraint::Length(7),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(10),
                Constraint::Min(16),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" services ({}) ", self.services.len())),
        )
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        let mut state = TableState::default()
            .with_selected((!self.services.is_empty()).then_some(self.selected));
        frame.render_stateful_widget(table, area, &mut state);
    }

    fn render_logs(&self, frame: &mut Frame<'_>, area: ratatui::layout::Rect) {
        let visible = usize::from(area.height.saturating_sub(2));
        let start = self.logs.len().saturating_sub(visible);
        let lines = self.logs[start..]
            .iter()
            .map(|entry| {
                let service = Span::styled(
                    format!("{:<10}", entry.service),
                    Style::default().fg(Color::Cyan),
                );
                let text_style = if entry.stream == "stderr" {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default()
                };
                Line::from(vec![service, Span::styled(entry.text.clone(), text_style)])
            })
            .collect::<Vec<_>>();
        let lines = if lines.is_empty() {
            vec![Line::from(Span::styled(
                "No log output yet.",
                Style::default().fg(Color::DarkGray),
            ))]
        } else {
            lines
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" logs · newest at the bottom ");
        frame.render_widget(
            Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn render_footer(&self, frame: &mut Frame<'_>, area: ratatui::layout::Rect) {
        let line = if self.message.is_empty() {
            Line::from(Span::styled(
                " q quit · ↑/↓ or j/k select · r restart · Ctrl+C quit ",
                Style::default().fg(Color::DarkGray),
            ))
        } else {
            Line::from(Span::styled(
                format!(" {} ", self.message),
                Style::default().fg(Color::Yellow),
            ))
        };
        frame.render_widget(Paragraph::new(line), area);
    }
}

fn status_color(status: &str) -> Color {
    match status {
        "healthy" => Color::Green,
        "starting" => Color::Yellow,
        "restarting" => Color::Yellow,
        "crashed" | "failed" => Color::Red,
        "stopped" | "exited" => Color::DarkGray,
        _ => Color::Blue,
    }
}

/// Formats bytes for the memory column.
pub fn format_memory(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{:.1}G", bytes as f64 / GIB as f64)
    } else {
        format!("{}M", bytes / MIB)
    }
}

/// Formats an uptime in seconds for the uptime column.
pub fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if days > 0 {
        format!("{days}d {hours:02}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::{format_memory, format_uptime, Dashboard, LogView, ServiceView};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn dashboard() -> Dashboard {
        let mut dashboard = Dashboard::new("shop");
        dashboard.daemon_running = true;
        dashboard.ready = true;
        dashboard.proxy_port = Some(80);
        dashboard.services = vec![
            ServiceView {
                name: "api".to_owned(),
                status: "healthy".to_owned(),
                port: 43123,
                cpu_percent: 1.5,
                memory_bytes: 48 * 1024 * 1024,
                restarts: 0,
                uptime_secs: 65,
                url: "http://127.0.0.1:43123".to_owned(),
                route_url: Some("http://api.localhost".to_owned()),
            },
            ServiceView {
                name: "web".to_owned(),
                status: "starting".to_owned(),
                port: 43124,
                cpu_percent: 0.0,
                memory_bytes: 96 * 1024 * 1024,
                restarts: 2,
                uptime_secs: 3_700,
                url: "http://127.0.0.1:43124".to_owned(),
                route_url: None,
            },
        ];
        dashboard.logs = vec![
            LogView {
                service: "api".to_owned(),
                stream: "stdout".to_owned(),
                text: "listening on 43123".to_owned(),
            },
            LogView {
                service: "web".to_owned(),
                stream: "stderr".to_owned(),
                text: "building".to_owned(),
            },
        ];
        dashboard
    }

    fn rendered(dashboard: &Dashboard) -> String {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| dashboard.render(frame))
            .expect("draw dashboard");
        let buffer = terminal.backend().buffer();
        buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn renders_services_logs_and_key_hints() {
        let text = rendered(&dashboard());
        for expected in [
            "Burrow · shop",
            "ready",
            "proxy :80",
            "api",
            "http://api.localhost",
            "web",
            "http://127.0.0.1:43124",
            "listening on 43123",
            "building",
            "q quit",
        ] {
            assert!(text.contains(expected), "missing `{expected}` in:\n{text}");
        }
    }

    #[test]
    fn selection_stays_inside_the_service_list() {
        let mut dashboard = dashboard();
        assert_eq!(
            dashboard
                .selected_service()
                .map(|service| service.name.as_str()),
            Some("api")
        );
        dashboard.move_selection(1);
        assert_eq!(
            dashboard
                .selected_service()
                .map(|service| service.name.as_str()),
            Some("web")
        );
        dashboard.move_selection(10);
        assert_eq!(dashboard.selected, 1);
        dashboard.move_selection(-10);
        assert_eq!(dashboard.selected, 0);

        dashboard.services.clear();
        dashboard.selected = 5;
        dashboard.clamp_selection();
        assert_eq!(dashboard.selected, 0);
        assert!(dashboard.selected_service().is_none());
    }

    #[test]
    fn renders_an_empty_project_without_panicking() {
        let mut dashboard = Dashboard::new("empty");
        dashboard.daemon_running = true;
        let text = rendered(&dashboard);
        assert!(text.contains("services (0)"), "text:\n{text}");
        assert!(text.contains("No log output yet."), "text:\n{text}");
    }

    #[test]
    fn formats_memory_and_uptime() {
        assert_eq!(format_memory(0), "0M");
        assert_eq!(format_memory(48 * 1024 * 1024), "48M");
        assert_eq!(format_memory(2 * 1024 * 1024 * 1024), "2.0G");
        assert_eq!(format_uptime(3), "3s");
        assert_eq!(format_uptime(65), "1m 05s");
        assert_eq!(format_uptime(3_700), "1h 01m");
        assert_eq!(format_uptime(90_000), "1d 01h");
    }
}
