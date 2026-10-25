//! The per-project daemon: orchestration, monitoring, and the control server.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use burrow_supervisor::{
    established_connections, wait_for_health, DirectoryWatcher, LogRing, ManagedProcess,
    RestartBackoff, RestartDecision,
};
use futures_util::future::join_all;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::control::{
    ControlCommand, ControlRequest, ControlResponse, LogEntry, MAX_REQUEST_BYTES,
};
use crate::project::{self, PreparedCommand, Project, ResolvedProject, ServicePlan};
use crate::state::{unix_now, RuntimeState, ServiceSnapshot, ServiceStatus, STATE_VERSION};

const TICK: Duration = Duration::from_millis(250);
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const STATE_WRITE_INTERVAL: Duration = Duration::from_secs(1);
/// How often idle-stop samples the connection table.
const IDLE_SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
/// How long watch events must stay quiet before a watched service restarts.
const WATCH_QUIESCE: Duration = Duration::from_millis(500);
/// Upper bound on the debounce window, so a steady stream of saves still
/// triggers a restart.
const WATCH_MAX_DELAY: Duration = Duration::from_secs(2);

enum Event {
    Health {
        name: String,
        generation: u64,
        result: std::result::Result<(), String>,
    },
    /// A proxy request asked for an idle service to be woken.
    WakeProxy {
        port: u16,
        reply: oneshot::Sender<std::result::Result<(), String>>,
    },
    /// A client connected to an idle service's wake listener.
    WakeConnect { name: String, client: TcpStream },
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
    /// When the connection sample last saw zero connections, for idle-stop.
    idle_since: Option<Instant>,
    /// When the connection table was last sampled for this service.
    idle_sampled: Option<Instant>,
    /// First pending watch event, for debouncing.
    watch_pending_since: Option<Instant>,
    /// Most recent watch event, for debouncing.
    watch_last_change: Option<Instant>,
    /// Proxy wake requests parked until the service is healthy again.
    wake_replies: Vec<oneshot::Sender<std::result::Result<(), String>>>,
    /// Accept task holding the wake listener while the service is idle.
    wake_task: Option<JoinHandle<()>>,
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
    event_tx: mpsc::Sender<Event>,
    /// Marks idle ports for the proxy and sends wake requests back.
    wake: burrow_net::proxy::ProxyWake,
    /// Wake listener clients parked until their service is healthy.
    wake_clients: BTreeMap<String, Vec<TcpStream>>,
    /// Directory watcher shared by every service with `watch` paths.
    watcher: Option<DirectoryWatcher>,
    /// Watch roots mapped to the services that declared them.
    watch_map: Vec<(std::path::PathBuf, String)>,
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

    // A data restore is a multi-directory swap. If the previous process died
    // mid-swap, roll it back before any service initializes over a missing
    // directory. `burrow restore` performs the same recovery before a retry.
    if let Some(recovery) = burrow_data::recover_interrupted_restore(&project.burrow_dir)
        .with_context(|| {
            format!(
                "could not recover an interrupted restore under {}",
                project.burrow_dir.join("data").display()
            )
        })?
    {
        warn!(
            committed = recovery.committed,
            services = ?recovery.services,
            "recovered an interrupted restore"
        );
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

    let (wake, mut wake_rx) = burrow_net::proxy::ProxyWake::new();

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
            let handle = burrow_net::proxy::start(routes, expose_proxy, port, Some(wake.clone()))
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
                idle_since: None,
                idle_sampled: None,
                watch_pending_since: None,
                watch_last_change: None,
                wake_replies: Vec::new(),
                wake_task: None,
            },
        );
    }

    let mut watch_map = Vec::new();
    let mut watch_roots = Vec::new();
    for (name, plan) in &resolved.services {
        for root in &plan.watch {
            watch_map.push((root.clone(), name.clone()));
            watch_roots.push(root.clone());
        }
    }
    let watcher = if watch_roots.is_empty() {
        None
    } else {
        match DirectoryWatcher::new(&watch_roots) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                warn!(%error, "file watching is unavailable; services run without watch restarts");
                None
            }
        }
    };

    let (event_tx, mut event_rx) = mpsc::channel::<Event>(64);

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
        event_tx: event_tx.clone(),
        wake,
        wake_clients: BTreeMap::new(),
        watcher,
        watch_map,
    };
    daemon.write_state()?;

    let (control_tx, mut control_rx) =
        mpsc::channel::<(ControlRequest, oneshot::Sender<ControlResponse>)>(32);
    let accept_task = tokio::spawn(accept_loop(listener, control_tx));

    let result = daemon
        .run_until_stopped(&mut control_rx, &mut event_rx, &mut wake_rx)
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
        wake_rx: &mut mpsc::UnboundedReceiver<burrow_net::proxy::WakeCall>,
    ) -> Result<()> {
        info!(
            project = %self.project.config.project.name,
            services = self.slots.len(),
            "starting Burrow"
        );
        self.start_all(&self.event_tx.clone()).await?;

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
                    let (response, shutdown) = self.handle_control(request, &self.event_tx.clone()).await;
                    let _ = reply.send(response);
                    if shutdown {
                        break;
                    }
                }
                Some(event) = event_rx.recv() => {
                    self.handle_event(event).await;
                }
                Some(call) = wake_rx.recv() => {
                    self.handle_event(Event::WakeProxy {
                        port: call.port,
                        reply: call.reply,
                    })
                    .await;
                }
                _ = ticker.tick() => {
                    self.tick(&self.event_tx.clone()).await;
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
            self.prepare_containers(layer).await;
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
        let mut to_idle_stop = Vec::new();
        let mut to_watch_restart = Vec::new();

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

        // Idle-stop samples the system connection table only for healthy,
        // running services of a fully started project.
        if self.ready {
            let now = Instant::now();
            for (name, slot) in &mut self.slots {
                let Some(idle_after) = slot.plan.idle_stop else {
                    continue;
                };
                if slot.status != ServiceStatus::Healthy || slot.process.is_none() {
                    slot.idle_since = None;
                    slot.idle_sampled = None;
                    continue;
                }
                if slot
                    .idle_sampled
                    .is_some_and(|sampled| sampled.elapsed() < IDLE_SAMPLE_INTERVAL)
                {
                    continue;
                }
                slot.idle_sampled = Some(now);
                match established_connections(slot.plan.port) {
                    Ok(count) if count > 0 => slot.idle_since = None,
                    Ok(_) => {
                        let since = *slot.idle_since.get_or_insert(now);
                        if now.saturating_duration_since(since) >= idle_after {
                            to_idle_stop.push(name.clone());
                        }
                    }
                    Err(error) => {
                        debug!(service = %name, %error, "connection sample failed")
                    }
                }
            }
        }

        // File watching: map fresh events to services, then debounce.
        if let Some(watcher) = &self.watcher {
            let events = watcher.poll();
            if !events.is_empty() {
                let now = Instant::now();
                let mut touched = BTreeSet::new();
                for event in events {
                    if is_ignored_watch_path(&event.relative) {
                        continue;
                    }
                    for (root, name) in &self.watch_map {
                        if &event.root == root {
                            touched.insert(name.clone());
                        }
                    }
                }
                for name in touched {
                    if let Some(slot) = self.slots.get_mut(&name) {
                        slot.watch_pending_since.get_or_insert(now);
                        slot.watch_last_change = Some(now);
                    }
                }
            }
        }
        let now = Instant::now();
        let mut to_rearm = Vec::new();
        for (name, slot) in &self.slots {
            if slot.watch_pending_since.is_none() {
                continue;
            }
            let (Some(pending_since), Some(last_change)) =
                (slot.watch_pending_since, slot.watch_last_change)
            else {
                continue;
            };
            let quiet = now.saturating_duration_since(last_change) >= WATCH_QUIESCE;
            let expired = now.saturating_duration_since(pending_since) >= WATCH_MAX_DELAY;
            if !quiet && !expired {
                continue;
            }
            let restartable = matches!(slot.status, ServiceStatus::Healthy | ServiceStatus::Idle)
                || (slot.status == ServiceStatus::Stopped && slot.desired);
            if restartable {
                to_watch_restart.push(name.clone());
            } else {
                // A service mid-restart re-arms its debounce window.
                to_rearm.push(name.clone());
            }
        }
        for name in to_rearm {
            if let Some(slot) = self.slots.get_mut(&name) {
                slot.watch_pending_since = Some(Instant::now());
            }
        }

        for name in to_restart {
            self.restart(&name, event_tx).await;
        }
        for name in to_idle_stop {
            self.idle_stop_service(&name).await;
        }
        for name in to_watch_restart {
            if let Some(slot) = self.slots.get_mut(&name) {
                slot.watch_pending_since = None;
                slot.watch_last_change = None;
            }
            self.restart_for_watch(&name, event_tx).await;
        }

        // Wake requests parked on a service that can no longer start get a
        // definitive failure instead of waiting out the proxy timeout.
        let failed_wakes: Vec<(String, String)> = self
            .slots
            .iter()
            .filter(|(_, slot)| {
                !slot.wake_replies.is_empty()
                    && matches!(slot.status, ServiceStatus::Failed | ServiceStatus::Stopped)
                    && slot.wake_task.is_none()
            })
            .map(|(name, slot)| {
                (
                    name.clone(),
                    slot.last_error
                        .clone()
                        .unwrap_or_else(|| "the service is not running".to_owned()),
                )
            })
            .collect();
        for (name, message) in failed_wakes {
            self.fail_wake(&name, message);
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
        if !self.slots.contains_key(name) {
            return;
        }
        {
            let slot = self.slots.get_mut(name).expect("slot checked above");
            if slot.process.is_some() {
                return;
            }
            // The wake listener must release the port before the service can
            // bind it again.
            if let Some(task) = slot.wake_task.take() {
                task.abort();
            }
            slot.generation = slot.generation.wrapping_add(1);
            slot.status = ServiceStatus::Starting;
            slot.restart_at = None;
            slot.exit_code = None;
            slot.restarts = slot.restarts.saturating_add(1);
            slot.desired = true;
        }
        let port = self.slots[name].plan.port;
        self.wake.mark_running(port);
        if self.slots[name].plan.container.is_some() {
            let layer = [name.to_owned()];
            self.prepare_containers(&layer).await;
        }

        let slot = self.slots.get_mut(name).expect("slot checked above");
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
            } else {
                self.flush_wake(name);
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
                } else {
                    self.flush_wake(&name);
                }
            }
            Event::WakeProxy { port, reply } => {
                let Some(name) = self
                    .slots
                    .iter()
                    .find(|(_, slot)| slot.plan.port == port)
                    .map(|(name, _)| name.clone())
                else {
                    let _ = reply.send(Err(format!("no service owns port {port}")));
                    return;
                };
                let slot = &self.slots[&name];
                match slot.status {
                    ServiceStatus::Healthy if slot.process.is_some() => {
                        self.wake.mark_running(port);
                        let _ = reply.send(Ok(()));
                    }
                    ServiceStatus::Starting | ServiceStatus::Restarting => {
                        self.slots
                            .get_mut(&name)
                            .expect("slot exists")
                            .wake_replies
                            .push(reply);
                    }
                    ServiceStatus::Idle | ServiceStatus::Stopped if slot.desired => {
                        info!(service = %name, "waking idle service for a proxied request");
                        self.slots
                            .get_mut(&name)
                            .expect("slot exists")
                            .wake_replies
                            .push(reply);
                        self.wake_service(&name).await;
                    }
                    _ => {
                        let _ = reply.send(Err(format!(
                            "service `{name}` is not running (status: {})",
                            slot.status.label()
                        )));
                    }
                }
            }
            Event::WakeConnect { name, client } => {
                let Some(slot) = self.slots.get(&name) else {
                    return;
                };
                match slot.status {
                    ServiceStatus::Healthy if slot.process.is_some() => {
                        let port = slot.plan.port;
                        tokio::spawn(pump_wake_client(client, port));
                    }
                    ServiceStatus::Starting | ServiceStatus::Restarting => {
                        self.wake_clients.entry(name).or_default().push(client);
                    }
                    ServiceStatus::Idle | ServiceStatus::Stopped if slot.desired => {
                        info!(service = %name, "waking idle service for a new connection");
                        self.wake_clients
                            .entry(name.clone())
                            .or_default()
                            .push(client);
                        self.wake_service(&name).await;
                    }
                    _ => {
                        // Not desired: the connection is closed by dropping it.
                        warn!(service = %name, "wake connection for a stopped service was closed");
                    }
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
        if !self.slots.contains_key(name) {
            return Ok(());
        }
        let port = self.slots[name].plan.port;
        {
            let slot = self.slots.get_mut(name).expect("slot checked above");
            slot.desired = false;
            slot.generation = slot.generation.wrapping_add(1);
            slot.restart_at = None;
            slot.idle_since = None;
            slot.idle_sampled = None;
            slot.watch_pending_since = None;
            slot.watch_last_change = None;
            // A stopped service must not hold its port through a wake listener.
            if let Some(task) = slot.wake_task.take() {
                task.abort();
            }
        }
        self.wake.mark_running(port);
        self.stop_process(name).await?;
        let slot = self.slots.get_mut(name).expect("slot checked above");
        slot.status = ServiceStatus::Stopped;
        self.dirty = true;
        Ok(())
    }

    /// Stops the process tree without changing desired state or status.
    ///
    /// Registry stop commands (for example `pg_ctl stop -m fast`) are what make
    /// databases flush cleanly; containers are stopped through the engine so
    /// `--rm` can clean up. Both run before terminating the job.
    async fn stop_process(&mut self, name: &str) -> Result<()> {
        let package_stop = self.slots[name]
            .plan
            .package
            .as_ref()
            .and_then(|package| package.stop.clone());
        let container = self.slots[name].plan.container.clone();
        let grace = self.slots[name].plan.spec.grace_period;
        let has_process = self.slots[name].process.is_some();

        if has_process {
            if let Some(command) = package_stop {
                if let Err(error) = self.run_package_step(name, &command, "stop").await {
                    warn!(
                        service = name,
                        %error,
                        "package stop command failed; terminating the process tree"
                    );
                }
            }
            if let Some(container) = container {
                let args = crate::container::stop_args(&container.container_name, grace.as_secs());
                info!(service = name, container = %container.container_name, "stopping container");
                match tokio::process::Command::new(&container.program)
                    .args(&args)
                    .stdin(Stdio::null())
                    .output()
                    .await
                {
                    Ok(output) if output.status.success() => {}
                    Ok(output) => warn!(
                        service = name,
                        code = output.status.code(),
                        "container stop command failed; terminating the engine client"
                    ),
                    Err(error) => {
                        warn!(service = name, %error, "could not run the container stop command")
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
            let slot = self.slots.get_mut(name).expect("slot checked above");
            slot.started = None;
            slot.started_wall = None;
            slot.memory_bytes = 0;
            slot.cpu_percent = 0.0;
            slot.sample = None;
        }
        Ok(())
    }

    /// Idle-stops a service: graceful stop, then hold its port with a wake
    /// listener so the next connection restarts it.
    async fn idle_stop_service(&mut self, name: &str) {
        let port = self.slots[name].plan.port;
        if let Err(error) = self.stop_process(name).await {
            warn!(service = name, %error, "idle stop failed; retrying on a later tick");
            if let Some(slot) = self.slots.get_mut(name) {
                slot.idle_since = Some(Instant::now());
            }
            return;
        }
        {
            let slot = self.slots.get_mut(name).expect("slot checked above");
            slot.status = ServiceStatus::Idle;
            slot.desired = true;
            slot.idle_since = None;
            slot.idle_sampled = None;
        }
        self.wake.mark_idle(port);
        info!(
            service = name,
            port, "service idle; stopped until the next connection"
        );
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => {
                let task = spawn_wake_listener(listener, name.to_owned(), self.event_tx.clone());
                if let Some(slot) = self.slots.get_mut(name) {
                    slot.wake_task = Some(task);
                }
            }
            Err(error) => {
                warn!(
                    service = name,
                    %error,
                    "could not bind the wake listener; the service stays idle without automatic wake-up"
                );
            }
        }
        self.dirty = true;
        self.write_state().ok();
    }

    /// Restarts an idle (or stopped-but-desired) service on demand.
    async fn wake_service(&mut self, name: &str) {
        if let Some(slot) = self.slots.get_mut(name) {
            if let Some(task) = slot.wake_task.take() {
                task.abort();
            }
            // Waking is deliberate, so it is not throttled by crash backoff.
            slot.backoff.reset();
            slot.desired = true;
        }
        let port = self.slots[name].plan.port;
        self.wake.mark_running(port);
        let event_tx = self.event_tx.clone();
        self.restart(name, &event_tx).await;
    }

    /// Answers parked wake requests and pumps parked clients for a service
    /// that just became healthy.
    fn flush_wake(&mut self, name: &str) {
        let Some(slot) = self.slots.get_mut(name) else {
            return;
        };
        let port = slot.plan.port;
        let replies = std::mem::take(&mut slot.wake_replies);
        self.wake.mark_running(port);
        for reply in replies {
            let _ = reply.send(Ok(()));
        }
        if let Some(clients) = self.wake_clients.remove(name) {
            for client in clients {
                tokio::spawn(pump_wake_client(client, port));
            }
        }
    }

    /// Fails parked wake requests after the service gave up starting.
    fn fail_wake(&mut self, name: &str, message: String) {
        let Some(slot) = self.slots.get_mut(name) else {
            return;
        };
        let replies = std::mem::take(&mut slot.wake_replies);
        for reply in replies {
            let _ = reply.send(Err(message.clone()));
        }
        self.wake_clients.remove(name);
    }

    /// Restarts only the watched service; dependents are left alone.
    async fn restart_for_watch(&mut self, name: &str, event_tx: &mpsc::Sender<Event>) {
        info!(
            service = name,
            "watched paths changed; restarting the service"
        );
        if self
            .slots
            .get(name)
            .is_some_and(|slot| slot.process.is_some())
        {
            if let Err(error) = self.stop_process(name).await {
                warn!(
                    service = name,
                    %error,
                    "could not stop the service before a watch restart"
                );
                return;
            }
        }
        self.restart(name, event_tx).await;
        self.dirty = true;
    }

    /// Removes containers left over by a crashed daemon, best effort.
    async fn prepare_containers(&self, layer: &[String]) {
        for name in layer {
            let Some(plan) = self.slots[name].plan.container.clone() else {
                continue;
            };
            let args = crate::container::remove_args(&plan.container_name);
            match tokio::process::Command::new(&plan.program)
                .args(&args)
                .stdin(Stdio::null())
                .output()
                .await
            {
                Ok(output) => {
                    debug!(
                        service = name,
                        container = %plan.container_name,
                        code = output.status.code(),
                        "cleared any leftover container"
                    );
                }
                Err(error) => warn!(service = name, %error, "could not clear a leftover container"),
            }
        }
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
            if let Some(quarantine) = burrow_data::quarantine_uninitialized(&package.data_dir)
                .map_err(|error| anyhow::anyhow!("service `{name}`: {error}"))?
            {
                warn!(
                    service = name,
                    path = %quarantine.display(),
                    "moved an uninitialized data directory aside before retrying init"
                );
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

        let working_directory = burrow_supervisor::process_working_directory(&slot.plan.cwd)?;
        let output = tokio::process::Command::new(&command.program)
            .args(&command.args)
            .current_dir(working_directory)
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
                        runtime: slot
                            .plan
                            .package
                            .as_ref()
                            .map(|package| package.key.clone())
                            .or_else(|| {
                                slot.plan
                                    .container
                                    .as_ref()
                                    .map(|container| format!("container:{}", container.image))
                            }),
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

/// Holds an idle service's port until the first connection wakes the service.
///
/// The listener is dropped as soon as one client arrives, which releases the
/// port so the real service can bind it again. That client is handed to the
/// daemon, which parks it until the service is healthy and then pumps bytes
/// between it and the service.
fn spawn_wake_listener(
    listener: TcpListener,
    name: String,
    event_tx: mpsc::Sender<Event>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        match listener.accept().await {
            Ok((client, _)) => {
                drop(listener);
                let _ = event_tx.send(Event::WakeConnect { name, client }).await;
            }
            Err(error) => warn!(%error, "wake listener failed; the port is now free"),
        }
    })
}

/// Bridges one wake-listener client to the freshly started service.
async fn pump_wake_client(mut client: TcpStream, port: u16) {
    let address = ("127.0.0.1", port);
    let mut upstream = None;
    for attempt in 0..10 {
        match TcpStream::connect(address).await {
            Ok(stream) => {
                upstream = Some(stream);
                break;
            }
            Err(_) if attempt < 9 => tokio::time::sleep(Duration::from_millis(200)).await,
            Err(_) => break,
        }
    }
    let Some(mut upstream) = upstream else {
        warn!(port, "a wake client could not reach the woken service");
        return;
    };
    if let Err(error) = tokio::io::copy_bidirectional(&mut client, &mut upstream).await {
        debug!(port, %error, "wake client tunnel ended");
    }
}

/// Watch events under Burrow's own state directory and `.git` never restart a
/// service: the first would loop forever (logs and data live there), the
/// second fires constantly while version control runs in the background.
fn is_ignored_watch_path(relative: &std::path::Path) -> bool {
    relative.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some(".burrow") | Some(".git")
        )
    })
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
