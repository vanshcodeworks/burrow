//! The per-project daemon: orchestration, monitoring, and the control server.

use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use burrow_supervisor::{
    wait_for_health, LogRing, ManagedProcess, RestartBackoff, RestartDecision,
};
use futures_util::future::join_all;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::control::{
    ControlCommand, ControlRequest, ControlResponse, LogEntry, MAX_REQUEST_BYTES,
};
use crate::project::{self, PreparedCommand, Project, ResolvedProject, ServicePlan};
use crate::state::{unix_now, RuntimeState, ServiceSnapshot, ServiceStatus, STATE_VERSION};

const TICK: Duration = Duration::from_millis(250);
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const STATE_WRITE_INTERVAL: Duration = Duration::from_secs(1);

enum Event {
    Health {
        name: String,
        generation: u64,
        result: std::result::Result<(), String>,
    },
}

struct Slot {
    plan: ServicePlan,
    logs: Arc<LogRing>,
    process: Option<ManagedProcess>,
    status: ServiceStatus,
    restarts: u32,
    started: Option<Instant>,
    started_wall: Option<u64>,
    exit_code: Option<i32>,
    last_error: Option<String>,
    backoff: RestartBackoff,
    restart_at: Option<Instant>,
    generation: u64,
    memory_bytes: u64,
    cpu_percent: f32,
    sample: Option<(Instant, Duration)>,
    desired: bool,
}

struct Daemon {
    project: Project,
    resolved: ResolvedProject,
    slots: BTreeMap<String, Slot>,
    token: String,
    control_port: u16,
    proxy: Option<burrow_net::proxy::ProxyHandle>,
    started_at: u64,
    ready: bool,
    last_state_write: Instant,
    dirty: bool,
}

/// Runs the daemon until it is asked to stop or Ctrl+C is pressed.
///
/// The caller must have approved the project's executable configuration.
pub async fn run(project: Project, selected: Vec<String>) -> Result<()> {
    if crate::control::load_live_state(&project.dir)
        .await?
        .is_some()
    {
        bail!(
            "Burrow is already running for this project\n\
             help: use `burrow status`, `burrow logs -f`, or `burrow down`"
        );
    }

    if let Some(stale) = RuntimeState::read(&project.dir)? {
        if crate::control::reachable(stale.control_port).await {
            bail!(
                "state exists for daemon pid {}, but it did not authenticate on 127.0.0.1:{}\n\
                 help: another process may be using that port; inspect {} before removing it",
                stale.pid,
                stale.control_port,
                RuntimeState::path(&project.dir).display()
            );
        }
        warn!(
            previous_pid = stale.pid,
            "removing stale state from a previous run"
        );
        RuntimeState::remove(&project.dir)?;
    }

    let mut allocator = project.ports()?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("could not bind the loopback control socket")?;
    let control_port = listener
        .local_addr()
        .context("could not read the control socket address")?
        .port();
    allocator.reserve(control_port);

    let resolved = project::resolve(&project, &selected, &mut allocator).await?;
    allocator.save()?;

    let routes = resolved
        .services
        .values()
        .filter_map(|plan| {
            plan.route.as_ref().map(|route| {
                (
                    route.clone(),
                    burrow_net::proxy::Route {
                        port: plan.port,
                        expose: plan.expose,
                    },
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let expose_proxy = resolved
        .services
        .values()
        .any(|plan| plan.route.is_some() && plan.expose);
    let proxy = match (resolved.proxy_port, routes.is_empty()) {
        (Some(port), false) => {
            if expose_proxy {
                warn!(
                    port,
                    "expose = true: the reverse proxy is bound to all interfaces; \
                     anything that can reach this machine can reach its routes"
                );
            }
            let handle = burrow_net::proxy::start(routes, expose_proxy, port)
                .await
                .with_context(|| format!("could not bind the reverse proxy on 127.0.0.1:{port}"))?;
            info!(
                port = handle.port(),
                exposed = handle.exposed(),
                "reverse proxy ready"
            );
            Some(handle)
        }
        _ => None,
    };

    std::fs::create_dir_all(project.logs_dir())
        .with_context(|| format!("could not create {}", project.logs_dir().display()))?;

    let token = hex::encode(rand::random::<[u8; 32]>());
    let started_at = unix_now();
    let mut slots = BTreeMap::new();
    for (name, plan) in resolved.services.iter() {
        let logs = Arc::new(LogRing::new(burrow_supervisor::DEFAULT_RING_CAPACITY));
        let mut plan = plan.clone();
        plan.spec.log_ring = Some(Arc::clone(&logs));
        slots.insert(
            name.clone(),
            Slot {
                plan,
                logs,
                process: None,
                status: ServiceStatus::Stopped,
                restarts: 0,
                started: None,
                started_wall: None,
                exit_code: None,
                last_error: None,
                backoff: RestartBackoff::default(),
                restart_at: None,
                generation: 0,
                memory_bytes: 0,
                cpu_percent: 0.0,
                sample: None,
                desired: true,
            },
        );
    }

    let mut daemon = Daemon {
        project,
        resolved,
        slots,
        token,
        control_port,
        proxy,
        started_at,
        ready: false,
        last_state_write: Instant::now(),
        dirty: true,
    };
    daemon.write_state()?;

    let (control_tx, mut control_rx) =
        mpsc::channel::<(ControlRequest, oneshot::Sender<ControlResponse>)>(32);
    let accept_task = tokio::spawn(accept_loop(listener, control_tx));
    let (event_tx, mut event_rx) = mpsc::channel::<Event>(64);

    let result = daemon
        .run_until_stopped(&mut control_rx, &mut event_rx, &event_tx)
        .await;

    daemon.ready = false;
    let stop_result = daemon.stop_all().await;
    RuntimeState::remove(&daemon.project.dir).ok();
    accept_task.abort();

    match (result, stop_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

impl Daemon {
    async fn run_until_stopped(
        &mut self,
        control_rx: &mut mpsc::Receiver<(ControlRequest, oneshot::Sender<ControlResponse>)>,
        event_rx: &mut mpsc::Receiver<Event>,
        event_tx: &mpsc::Sender<Event>,
    ) -> Result<()> {
        info!(
            project = %self.project.config.project.name,
            services = self.slots.len(),
            "starting Burrow"
        );
        self.start_all(event_tx).await?;

        self.ready = true;
        self.dirty = true;
        self.write_state()?;
        for slot in self.slots.values() {
            info!(service = %slot.plan.name, port = slot.plan.port, url = %slot.plan.url, "service ready");
        }
        info!("all services are ready; press Ctrl+C or run `burrow down` to stop");

        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);

        loop {
            tokio::select! {
                _ = &mut ctrl_c => {
                    info!("Ctrl+C received; stopping services");
                    break;
                }
                Some((request, reply)) = control_rx.recv() => {
                    let (response, shutdown) = self.handle_control(request, event_tx).await;
                    let _ = reply.send(response);
                    if shutdown {
                        break;
                    }
                }
                Some(event) = event_rx.recv() => {
                    self.handle_event(event).await;
                }
                _ = ticker.tick() => {
                    self.tick(event_tx).await;
                    if self.last_state_write.elapsed() >= STATE_WRITE_INTERVAL {
                        self.write_state()?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn start_all(&mut self, event_tx: &mpsc::Sender<Event>) -> Result<()> {
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        let layers = self.resolved.startup.layers.clone();

        for (index, layer) in layers.iter().enumerate() {
            debug!(layer = index, services = ?layer, "starting layer");
            self.initialize_packages(layer).await?;
            let spawns = layer.iter().map(|name| {
                let slot = &self.slots[name];
                let mut spec = slot.plan.spec.clone();
                spec.log_ring = Some(Arc::clone(&slot.logs));
                let name = name.clone();
                async move { (name, ManagedProcess::spawn(spec).await) }
            });

            let results = tokio::select! {
                _ = &mut ctrl_c => bail!("startup interrupted by Ctrl+C"),
                results = join_all(spawns) => results,
            };

            for (name, result) in results {
                match result {
                    Ok(process) => {
                        let slot = self.slots.get_mut(&name).expect("planned service");
                        info!(service = %name, pid = process.pid(), "process started");
                        debug!(
                            service = %name,
                            command = %slot.plan.command_display,
                            "starting configured command"
                        );
                        slot.process = Some(process);
                        slot.started = Some(Instant::now());
                        slot.started_wall = Some(unix_now());
                        slot.status = ServiceStatus::Starting;
                    }
                    Err(error) => {
                        let slot = self.slots.get_mut(&name).expect("planned service");
                        slot.status = ServiceStatus::Failed;
                        slot.last_error = Some(format!("{error}"));
                        self.dirty = true;
                        self.write_state()?;
                        bail!("could not start service `{name}`: {error}");
                    }
                }
            }

            let healths = layer.iter().filter_map(|name| {
                let slot = &self.slots[name];
                slot.plan
                    .health
                    .as_ref()
                    .map(|health| (name.clone(), health.check.clone(), health.options))
            });
            let checks = healths.map(|(name, check, options)| async move {
                (name, wait_for_health(&check, options).await)
            });
            let results = tokio::select! {
                _ = &mut ctrl_c => bail!("startup interrupted by Ctrl+C"),
                results = join_all(checks) => results,
            };

            for (name, result) in results {
                match result {
                    Ok(report) => {
                        let slot = self.slots.get_mut(&name).expect("planned service");
                        slot.status = ServiceStatus::Healthy;
                        slot.backoff.mark_started();
                        info!(
                            service = %name,
                            attempts = report.attempts,
                            elapsed_ms = report.elapsed.as_millis(),
                            "health check passed"
                        );
                    }
                    Err(error) => {
                        let slot = self.slots.get_mut(&name).expect("planned service");
                        slot.status = ServiceStatus::Failed;
                        slot.last_error = Some(error.to_string());
                        self.dirty = true;
                        self.write_state()?;
                        bail!(
                            "service `{name}` did not become healthy: {error}\n\
                             help: see {}\n\
                             help: see the daemon log at {}",
                            self.project
                                .logs_dir()
                                .join(format!("{name}.log"))
                                .display(),
                            self.project.burrow_dir.join("daemon.log").display()
                        );
                    }
                }
            }

            for name in layer {
                if self.slots[name].plan.health.is_none() {
                    self.slots.get_mut(name).expect("planned service").status =
                        ServiceStatus::Healthy;
                    self.slots
                        .get_mut(name)
                        .expect("planned service")
                        .backoff
                        .mark_started();
                }
            }
            for name in layer {
                if let Err(error) = self.seed_if_needed(name).await {
                    let slot = self.slots.get_mut(name).expect("planned service");
                    slot.status = ServiceStatus::Failed;
                    slot.last_error = Some(format!("{error}"));
                    self.dirty = true;
                    self.write_state()?;
                    bail!("{error}");
                }
            }
            self.dirty = true;
            self.write_state()?;
        }

        let _ = event_tx;
        Ok(())
    }

    async fn tick(&mut self, event_tx: &mpsc::Sender<Event>) {
        let mut to_restart = Vec::new();

        for (name, slot) in &mut self.slots {
            if let Some(process) = &slot.process {
                if let Some(status) = process.try_wait() {
                    let uptime = slot
                        .started
                        .map(|started| started.elapsed())
                        .unwrap_or_default();
                    let mut process = slot.process.take().expect("process checked above");
                    // The direct child exited; this also removes descendants
                    // that stayed in the job.
                    let _ = process.shutdown(Some(Duration::ZERO)).await;
                    slot.started = None;
                    slot.started_wall = None;
                    slot.memory_bytes = 0;
                    slot.cpu_percent = 0.0;
                    slot.sample = None;
                    slot.exit_code = status.code();
                    self.dirty = true;

                    if !slot.desired {
                        slot.status = ServiceStatus::Stopped;
                        continue;
                    }

                    if slot.plan.restart.should_restart(status.success()) {
                        Self::schedule_restart(slot, uptime, format!("exited with {status}"));
                    } else {
                        slot.status = ServiceStatus::Stopped;
                        info!(service = %name, %status, "service exited; restart policy is `never` or exit was clean");
                    }
                }
            }

            if slot.status == ServiceStatus::Restarting
                && slot.restart_at.is_some_and(|at| Instant::now() >= at)
            {
                to_restart.push(name.clone());
            }
        }

        for name in to_restart {
            self.restart(&name, event_tx).await;
        }

        self.sample_metrics();
    }

    fn schedule_restart(slot: &mut Slot, uptime: Duration, reason: String) {
        match slot.backoff.on_failure(uptime) {
            RestartDecision::RestartAfter(delay) => {
                slot.status = ServiceStatus::Restarting;
                slot.restart_at = Some(Instant::now() + delay);
                slot.last_error = Some(format!(
                    "{reason}; retrying in {}",
                    humantime::format_duration(delay)
                ));
                warn!(
                    service = %slot.plan.name,
                    delay_ms = delay.as_millis(),
                    %reason,
                    "scheduling restart"
                );
            }
            RestartDecision::CrashLoop { failures, window } => {
                slot.status = ServiceStatus::Failed;
                slot.restart_at = None;
                slot.last_error = Some(format!(
                    "crash loop: {failures} failures within {}; not restarting",
                    humantime::format_duration(window)
                ));
                warn!(
                    service = %slot.plan.name,
                    failures,
                    window_secs = window.as_secs(),
                    "crash-loop breaker tripped"
                );
            }
        }
    }

    async fn restart(&mut self, name: &str, event_tx: &mpsc::Sender<Event>) {
        let Some(slot) = self.slots.get_mut(name) else {
            return;
        };
        if slot.process.is_some() {
            return;
        }
        slot.generation = slot.generation.wrapping_add(1);
        slot.status = ServiceStatus::Starting;
        slot.restart_at = None;
        slot.exit_code = None;
        slot.restarts = slot.restarts.saturating_add(1);
        slot.desired = true;

        let mut spec = slot.plan.spec.clone();
        spec.log_ring = Some(Arc::clone(&slot.logs));
        let mut seed_now = false;
        match ManagedProcess::spawn(spec).await {
            Ok(process) => {
                info!(service = %name, pid = process.pid(), "process restarted");
                slot.process = Some(process);
                slot.started = Some(Instant::now());
                slot.started_wall = Some(unix_now());
                slot.last_error = None;
                slot.sample = None;
                self.dirty = true;

                if let Some(health) = &slot.plan.health {
                    let generation = slot.generation;
                    let check = health.check.clone();
                    let options = health.options;
                    spawn_health_check(
                        event_tx.clone(),
                        name.to_owned(),
                        generation,
                        check,
                        options,
                    );
                } else {
                    slot.status = ServiceStatus::Healthy;
                    slot.backoff.mark_started();
                    seed_now = true;
                }
            }
            Err(error) => {
                slot.exit_code = None;
                Self::schedule_restart(slot, Duration::ZERO, format!("could not start: {error}"));
            }
        }

        if seed_now {
            if let Err(error) = self.seed_if_needed(name).await {
                if let Some(slot) = self.slots.get_mut(name) {
                    if let Some(mut process) = slot.process.take() {
                        let _ = process.shutdown(None).await;
                    }
                    slot.started = None;
                    slot.started_wall = None;
                    Self::schedule_restart(slot, Duration::ZERO, format!("{error}"));
                }
                self.dirty = true;
            }
        }
    }

    async fn handle_event(&mut self, event: Event) {
        match event {
            Event::Health {
                name,
                generation,
                result,
            } => {
                {
                    let Some(slot) = self.slots.get_mut(&name) else {
                        return;
                    };
                    if slot.generation != generation || slot.status != ServiceStatus::Starting {
                        return;
                    }
                    match result {
                        Ok(()) => {
                            slot.status = ServiceStatus::Healthy;
                            slot.last_error = None;
                            slot.backoff.mark_started();
                            info!(service = %name, "health check passed after restart");
                        }
                        Err(message) => {
                            if let Some(mut process) = slot.process.take() {
                                let _ = process.shutdown(None).await;
                            }
                            slot.started = None;
                            slot.started_wall = None;
                            let uptime = Duration::ZERO;
                            Self::schedule_restart(
                                slot,
                                uptime,
                                format!("health check failed: {message}"),
                            );
                            self.dirty = true;
                            return;
                        }
                    }
                }
                self.dirty = true;

                if let Err(error) = self.seed_if_needed(&name).await {
                    if let Some(slot) = self.slots.get_mut(&name) {
                        if let Some(mut process) = slot.process.take() {
                            let _ = process.shutdown(None).await;
                        }
                        slot.started = None;
                        slot.started_wall = None;
                        Self::schedule_restart(slot, Duration::ZERO, format!("{error}"));
                    }
                    self.dirty = true;
                }
            }
        }
    }

    async fn handle_control(
        &mut self,
        request: ControlRequest,
        event_tx: &mpsc::Sender<Event>,
    ) -> (ControlResponse, bool) {
        if request.version != STATE_VERSION {
            return (
                ControlResponse::error("the daemon and CLI versions do not match"),
                false,
            );
        }
        if request.token != self.token {
            return (
                ControlResponse::error("unauthorized: the state file token did not match"),
                false,
            );
        }

        match request.command {
            ControlCommand::Ping => (ControlResponse::ok(self.snapshot(), "pong"), false),
            ControlCommand::Status => (ControlResponse::ok(self.snapshot(), ""), false),
            ControlCommand::Logs {
                service,
                since_seq,
                limit,
            } => {
                let mut entries = Vec::new();
                for (name, slot) in &self.slots {
                    if service.as_deref().is_some_and(|wanted| wanted != name) {
                        continue;
                    }
                    for line in slot.logs.snapshot(since_seq, 0) {
                        entries.push(LogEntry::from_line(name, &line));
                    }
                }
                entries.sort_by_key(|entry| entry.at_unix_ms);
                if limit > 0 && entries.len() > limit {
                    entries = entries.split_off(entries.len() - limit);
                }
                (ControlResponse::logs(self.snapshot(), entries), false)
            }
            ControlCommand::Restart { service } => {
                if !self.slots.contains_key(&service) {
                    return (
                        ControlResponse::error(format!("unknown service `{service}`")),
                        false,
                    );
                }
                let (stop_order, start_order) =
                    match project::restart_order(&self.project.config, &self.resolved, &service) {
                        Ok(order) => order,
                        Err(error) => return (ControlResponse::error(format!("{error}")), false),
                    };

                for name in &stop_order {
                    if let Err(error) = self.stop_slot(name).await {
                        return (
                            ControlResponse::error(format!(
                                "could not stop `{name}` before restarting: {error}"
                            )),
                            false,
                        );
                    }
                }
                for name in &stop_order {
                    if let Some(slot) = self.slots.get_mut(name) {
                        slot.backoff.reset();
                        slot.desired = true;
                    }
                }
                for name in &start_order {
                    self.restart(name, event_tx).await;
                }
                self.dirty = true;
                self.write_state().ok();
                (
                    ControlResponse::ok(
                        self.snapshot(),
                        format!("restarting {}", start_order.join(", ")),
                    ),
                    false,
                )
            }
            ControlCommand::Shutdown => {
                self.ready = false;
                self.dirty = true;
                self.write_state().ok();
                (ControlResponse::ok(self.snapshot(), "stopping"), true)
            }
        }
    }

    async fn stop_slot(&mut self, name: &str) -> Result<()> {
        let Some(slot) = self.slots.get_mut(name) else {
            return Ok(());
        };
        slot.desired = false;
        slot.generation = slot.generation.wrapping_add(1);
        slot.restart_at = None;
        let stop = slot
            .plan
            .package
            .as_ref()
            .and_then(|package| package.stop.clone());
        let has_process = slot.process.is_some();

        // Registry stop commands (for example `pg_ctl stop -m fast`) are what
        // make databases flush cleanly. Running them before terminating the job
        // avoids an unclean shutdown every time the project stops.
        if has_process {
            if let Some(command) = stop {
                if let Err(error) = self.run_package_step(name, &command, "stop").await {
                    warn!(
                        service = name,
                        %error,
                        "package stop command failed; terminating the process tree"
                    );
                }
            }
        }

        let slot = self.slots.get_mut(name).expect("slot checked above");
        if let Some(mut process) = slot.process.take() {
            process
                .shutdown(None)
                .await
                .with_context(|| format!("could not stop service `{name}` cleanly"))?;
        }
        slot.status = ServiceStatus::Stopped;
        slot.started = None;
        slot.started_wall = None;
        slot.memory_bytes = 0;
        slot.cpu_percent = 0.0;
        slot.sample = None;
        self.dirty = true;
        Ok(())
    }

    /// Runs the registry `init` commands once per data directory.
    async fn initialize_packages(&self, layer: &[String]) -> Result<()> {
        for name in layer {
            let Some(package) = self.slots[name].plan.package.clone() else {
                continue;
            };
            if !burrow_data::needs_init(&package.data_dir) {
                continue;
            }
            if let Some(parent) = package.data_dir.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("could not create {}", parent.display()))?;
            }
            info!(
                service = name,
                package = %package.key,
                data_dir = %package.data_dir.display(),
                "initializing package data directory"
            );
            for command in &package.init {
                self.run_package_step(name, command, "init").await?;
            }
            burrow_data::mark_initialized(&package.data_dir)
                .map_err(|error| anyhow::anyhow!("service `{name}`: {error}"))?;
            info!(service = name, package = %package.key, "package initialized");
        }
        Ok(())
    }

    /// Runs the seed script after the service is healthy, exactly once.
    async fn seed_if_needed(&self, name: &str) -> Result<()> {
        let Some(package) = self.slots[name].plan.package.clone() else {
            return Ok(());
        };
        if !burrow_data::needs_seed(&package.data_dir) {
            return Ok(());
        }
        let (Some(command), Some(hash)) = (package.seed.clone(), package.seed_sha256.clone())
        else {
            return Ok(());
        };
        info!(
            service = name,
            package = %package.key,
            seed = %package
                .seed_file
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            "seeding package data directory"
        );
        self.run_package_step(name, &command, "seed").await?;
        burrow_data::mark_seeded(&package.data_dir, &hash)
            .map_err(|error| anyhow::anyhow!("service `{name}`: {error}"))?;
        info!(service = name, package = %package.key, "package seeded");
        Ok(())
    }

    /// Runs one registry command, capturing its output in the service log ring.
    async fn run_package_step(
        &self,
        name: &str,
        command: &PreparedCommand,
        phase: &str,
    ) -> Result<()> {
        let slot = self
            .slots
            .get(name)
            .with_context(|| format!("unknown service `{name}`"))?;
        slot.logs
            .push("stdout", format!("[burrow] {phase}: {}", command.display));
        info!(
            service = name,
            phase,
            command = %command.display,
            "running package step"
        );

        let output = tokio::process::Command::new(&command.program)
            .args(&command.args)
            .current_dir(&slot.plan.cwd)
            .envs(&slot.plan.env)
            .stdin(Stdio::null())
            .output()
            .await
            .with_context(|| format!("could not run the `{phase}` command for service `{name}`"))?;

        let mut transcript = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            slot.logs.push("stdout", format!("[{phase}] {line}"));
            transcript.push(line.to_owned());
        }
        for line in String::from_utf8_lossy(&output.stderr).lines() {
            slot.logs.push("stderr", format!("[{phase}] {line}"));
            transcript.push(line.to_owned());
        }

        if !output.status.success() {
            let start = transcript.len().saturating_sub(20);
            let tail = transcript[start..].join("\n");
            bail!(
                "service `{name}` failed during `{phase}` ({})\n\
                 command: {}\n\
                 output:\n{tail}",
                output.status,
                command.display
            );
        }
        Ok(())
    }

    async fn stop_all(&mut self) -> Result<()> {
        let mut order = self.resolved.startup.order();
        order.reverse();
        let mut failures = Vec::new();
        for name in order {
            if let Err(error) = self.stop_slot(&name).await {
                failures.push(format!("{name}: {error}"));
            }
        }
        self.dirty = true;
        self.write_state().ok();
        if failures.is_empty() {
            info!("all services stopped");
            Ok(())
        } else {
            bail!(
                "some services did not stop cleanly: {}",
                failures.join("; ")
            )
        }
    }

    fn sample_metrics(&mut self) {
        for slot in self.slots.values_mut() {
            let Some(process) = &slot.process else {
                continue;
            };
            if slot
                .sample
                .is_some_and(|(sampled, _)| sampled.elapsed() < SAMPLE_INTERVAL)
            {
                continue;
            }
            let Ok(stats) = process.stats() else {
                continue;
            };
            if let Some((sampled, previous_cpu)) = slot.sample {
                let wall = sampled.elapsed().as_secs_f64();
                let used = stats.cpu_time.saturating_sub(previous_cpu).as_secs_f64();
                slot.cpu_percent = if wall > 0.0 {
                    (used / wall * 100.0) as f32
                } else {
                    0.0
                };
            }
            slot.memory_bytes = stats.working_set_bytes;
            slot.sample = Some((Instant::now(), stats.cpu_time));
            self.dirty = true;
        }
    }

    fn snapshot(&self) -> RuntimeState {
        let proxy_port = self
            .proxy
            .as_ref()
            .map(burrow_net::proxy::ProxyHandle::port);
        let services = self
            .slots
            .iter()
            .map(|(name, slot)| {
                let route_url = slot.plan.route.as_ref().and_then(|route| {
                    proxy_port.map(|port| burrow_net::proxy::url_for(route, port))
                });
                (
                    name.clone(),
                    ServiceSnapshot {
                        status: slot.status,
                        pid: slot.process.as_ref().map(ManagedProcess::pid),
                        port: slot.plan.port,
                        url: slot.plan.url.clone(),
                        route: slot.plan.route.clone(),
                        route_url,
                        restarts: slot.restarts,
                        started_at: slot.started_wall,
                        exit_code: slot.exit_code,
                        memory_bytes: slot.memory_bytes,
                        cpu_percent: slot.cpu_percent,
                        last_error: slot.last_error.clone(),
                    },
                )
            })
            .collect();

        RuntimeState {
            version: STATE_VERSION,
            project: self.project.config.project.name.clone(),
            project_dir: self.project.dir.clone(),
            pid: std::process::id(),
            token: self.token.clone(),
            control_port: self.control_port,
            proxy_port,
            started_at: self.started_at,
            config_hash: self.project.config_hash.clone(),
            ready: self.ready,
            services,
        }
    }

    fn write_state(&mut self) -> Result<()> {
        self.snapshot().write(&self.project.dir)?;
        self.last_state_write = Instant::now();
        self.dirty = false;
        Ok(())
    }
}

fn spawn_health_check(
    event_tx: mpsc::Sender<Event>,
    name: String,
    generation: u64,
    check: burrow_supervisor::HealthCheck,
    options: burrow_supervisor::HealthOptions,
) {
    tokio::spawn(async move {
        let result = wait_for_health(&check, options)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string());
        let _ = event_tx
            .send(Event::Health {
                name,
                generation,
                result,
            })
            .await;
    });
}

async fn accept_loop(
    listener: TcpListener,
    sender: mpsc::Sender<(ControlRequest, oneshot::Sender<ControlResponse>)>,
) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(pair) => pair,
            Err(error) => {
                warn!(%error, "control socket accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let sender = sender.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_connection(stream, sender).await {
                debug!(%error, "control connection ended with an error");
            }
        });
    }
}

async fn serve_connection(
    stream: TcpStream,
    sender: mpsc::Sender<(ControlRequest, oneshot::Sender<ControlResponse>)>,
) -> Result<()> {
    stream.set_nodelay(true).ok();
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    {
        let mut limited = (&mut reader).take(MAX_REQUEST_BYTES as u64 + 1);
        tokio::time::timeout(
            Duration::from_secs(10),
            limited.read_until(b'\n', &mut line),
        )
        .await
        .context("timed out reading the control request")??;
    }
    if line.len() > MAX_REQUEST_BYTES {
        return write_response(
            reader.into_inner(),
            ControlResponse::error("request too large"),
        )
        .await;
    }

    let text = String::from_utf8_lossy(&line);
    let request: ControlRequest = match serde_json::from_str(text.trim()) {
        Ok(request) => request,
        Err(error) => {
            return write_response(
                reader.into_inner(),
                ControlResponse::error(format!("invalid control request: {error}")),
            )
            .await
        }
    };

    let (reply_tx, reply_rx) = oneshot::channel();
    if sender.send((request, reply_tx)).await.is_err() {
        return write_response(
            reader.into_inner(),
            ControlResponse::error("the daemon is shutting down"),
        )
        .await;
    }
    let response = tokio::time::timeout(Duration::from_secs(120), reply_rx)
        .await
        .ok()
        .and_then(std::result::Result::ok)
        .unwrap_or_else(|| ControlResponse::error("the daemon did not handle the request"));
    write_response(reader.into_inner(), response).await
}

async fn write_response(mut stream: TcpStream, response: ControlResponse) -> Result<()> {
    let mut encoded = serde_json::to_vec(&response).context("could not encode the response")?;
    encoded.push(b'\n');
    stream
        .write_all(&encoded)
        .await
        .context("could not write the control response")?;
    stream.flush().await.ok();
    Ok(())
}
