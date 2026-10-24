//! Project discovery, trust approval, secrets, and runtime planning.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use burrow_config::{Config, HealthKind, RestartPolicy, Service, StartupPlan, LOCAL_CONFIG_FILE};
use burrow_net::PortAllocator;
use burrow_pkg::{
    Cache, DownloadProgress, InitStep, PackageHealth, PackageRef, Registry, TemplateCommand,
};
use burrow_supervisor::{HealthCheck, HealthOptions, ProcessSpec};
use serde::{Deserialize, Serialize};

use crate::commandline;
use crate::state::BURROW_DIR;

const SECRETS_FILE: &str = "secrets";
const TRUST_FILE: &str = "trust.json";
const TRUST_VERSION: u32 = 1;

/// A loaded project rooted at an absolute directory.
#[derive(Debug, Clone)]
pub struct Project {
    /// Absolute project directory.
    pub dir: PathBuf,
    /// `.burrow` directory.
    pub burrow_dir: PathBuf,
    /// Parsed and validated configuration.
    pub config: Config,
    /// Hash that gates the trust prompt.
    pub config_hash: String,
}

impl Project {
    /// Loads `burrow.toml` from `directory` or the current directory.
    pub fn load(directory: Option<PathBuf>) -> Result<Self> {
        let directory = directory.unwrap_or(
            std::env::current_dir()
                .context("could not determine the current directory; pass --project <DIR>")?,
        );
        let dir = directory.canonicalize().with_context(|| {
            format!(
                "could not open project directory {}; check that it exists",
                directory.display()
            )
        })?;
        // `canonicalize` returns a verbatim `\\?\` path. Child processes and
        // `cmd.exe` handle the plain drive form better, so strip the prefix.
        let dir = simplify_windows_path(dir);

        let config = burrow_config::load(&dir).map_err(|error| {
            anyhow::anyhow!(
                "{error}\nnote: expected burrow.toml in {}\nhelp: run `burrow init` to create one",
                dir.display()
            )
        })?;
        let config_hash = config.executable_hash();
        let burrow_dir = dir.join(BURROW_DIR);

        Ok(Self {
            dir,
            burrow_dir,
            config,
            config_hash,
        })
    }

    /// Directory for rotating service log files.
    pub fn logs_dir(&self) -> PathBuf {
        self.burrow_dir.join("logs")
    }

    /// Data directory for a service.
    pub fn data_dir(&self, service: &str) -> PathBuf {
        self.burrow_dir.join("data").join(service)
    }

    /// Opens the per-project port allocator.
    pub fn ports(&self) -> Result<PortAllocator> {
        PortAllocator::load(&self.burrow_dir).map_err(Into::into)
    }
}

/// Strips the verbatim `\\?\` prefix from a canonical Windows path.
#[cfg(windows)]
fn simplify_windows_path(path: PathBuf) -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    const PREFIX: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const UNC: [u16; 4] = [b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16];
    if !wide.starts_with(&PREFIX) {
        return path;
    }
    let rest = &wide[PREFIX.len()..];
    if rest.starts_with(&UNC) {
        let mut restored = vec![b'\\' as u16, b'\\' as u16];
        restored.extend_from_slice(&rest[UNC.len()..]);
        return PathBuf::from(OsString::from_wide(&restored));
    }
    PathBuf::from(OsString::from_wide(rest))
}

#[cfg(not(windows))]
fn simplify_windows_path(path: PathBuf) -> PathBuf {
    path
}

/// A fully resolved service, ready to start.
#[derive(Clone)]
pub struct ServicePlan {
    /// Service name.
    pub name: String,
    /// Process specification.
    pub spec: ProcessSpec,
    /// Allocated port.
    pub port: u16,
    /// Generated URL.
    pub url: String,
    /// Reverse-proxy hostname.
    pub route: Option<String>,
    /// Whether the service explicitly opted into non-loopback access.
    pub expose: bool,
    /// Health check, if configured.
    pub health: Option<HealthSettings>,
    /// Restart policy.
    pub restart: RestartPolicy,
    /// Command shown in summaries. Never includes environment values.
    pub command_display: String,
    /// Resolved working directory. Used by `burrow env` and `burrow exec`.
    #[allow(dead_code)]
    pub cwd: PathBuf,
    /// Resolved environment (also present in the process specification). Used
    /// by `burrow env` and `burrow exec`.
    #[allow(dead_code)]
    pub env: BTreeMap<OsString, OsString>,
    /// Managed package runtime, when this service uses `use = "..."`.
    pub package: Option<PackagePlan>,
}

/// A registry command with every placeholder already substituted.
#[derive(Clone)]
pub struct PreparedCommand {
    /// Absolute executable path inside the package root.
    pub program: PathBuf,
    /// Arguments, with no shell involved.
    pub args: Vec<OsString>,
    /// Human-readable form with credentials redacted.
    pub display: String,
}

/// Everything the daemon needs to initialize and seed a managed package.
#[derive(Clone)]
pub struct PackagePlan {
    /// Canonical `name@version`, for diagnostics.
    pub key: String,
    /// Per-project data directory.
    pub data_dir: PathBuf,
    /// Initialization commands, in registry order.
    pub init: Vec<PreparedCommand>,
    /// Seed command, when both the registry and the config define one.
    pub seed: Option<PreparedCommand>,
    /// Seed script path, kept for diagnostics.
    #[allow(dead_code)]
    pub seed_file: Option<PathBuf>,
    /// SHA-256 of the seed script at plan time.
    pub seed_sha256: Option<String>,
    /// Graceful stop command, when the package defines one.
    pub stop: Option<PreparedCommand>,
}

/// One installed package selected from the registry.
struct InstalledPackage {
    key: String,
    root: PathBuf,
    url_template: Option<String>,
    run: TemplateCommand,
    stop: Option<TemplateCommand>,
    init: Vec<InitStep>,
    seed: Option<TemplateCommand>,
    health: Option<PackageHealth>,
    binaries: BTreeMap<String, String>,
}

/// Health settings bound to an allocated port.
#[derive(Clone)]
pub struct HealthSettings {
    /// The check to run.
    pub check: HealthCheck,
    /// Retry and timeout options.
    pub options: HealthOptions,
}

/// All services selected for this daemon, plus their startup order.
pub struct ResolvedProject {
    /// Services keyed by name.
    pub services: BTreeMap<String, ServicePlan>,
    /// Layered startup plan restricted to selected services and dependencies.
    pub startup: StartupPlan,
    /// Port reserved for the reverse proxy, when at least one route exists.
    pub proxy_port: Option<u16>,
}

/// Port key used for the shared reverse proxy in `.burrow/ports.json`.
pub const PROXY_PORT_KEY: &str = "__proxy__";
/// Preferred proxy port. Port 80 is legal for unprivileged Windows users and
/// keeps `http://shop.localhost` free of a port suffix.
pub const DEFAULT_PROXY_PORT: u16 = 80;

fn allocate_proxy_port(allocator: &mut PortAllocator) -> Result<u16> {
    if let Some(port) = allocator.pinned(PROXY_PORT_KEY) {
        return Ok(port);
    }
    match allocator.allocate(PROXY_PORT_KEY, Some(DEFAULT_PROXY_PORT)) {
        Ok(port) => Ok(port),
        Err(_) => allocator
            .allocate(PROXY_PORT_KEY, None)
            .context("could not reserve a port for the reverse proxy"),
    }
}

/// Loads or generates the per-project secret for every service.
///
/// Secrets are written to `.burrow/secrets` (gitignored) and are never printed.
pub fn load_or_create_secrets(project: &Project) -> Result<BTreeMap<String, String>> {
    let path = project.burrow_dir.join(SECRETS_FILE);
    let mut document = match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str::<SecretsFile>(&contents)
            .with_context(|| format!("{} is not valid Burrow secrets", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SecretsFile::default(),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()))
        }
    };

    let mut changed = false;
    for name in project.config.services.keys() {
        document.secrets.entry(name.clone()).or_insert_with(|| {
            changed = true;
            hex::encode(rand::random::<[u8; 24]>())
        });
    }

    if changed || !path.exists() {
        std::fs::create_dir_all(&project.burrow_dir)
            .with_context(|| format!("could not create {}", project.burrow_dir.display()))?;
        let temporary = path.with_extension("tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(&document).context("could not encode secrets")?,
        )
        .with_context(|| format!("could not write {}", temporary.display()))?;
        std::fs::rename(&temporary, &path)
            .with_context(|| format!("could not replace {}", path.display()))?;
    }

    Ok(document.secrets)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SecretsFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}

/// Requires an explicit approval for this project's executable configuration.
///
/// The approval is keyed by [`Config::executable_hash`], so changing a `run` or
/// `use` entry re-prompts, while changing ports or memory limits does not.
pub fn ensure_trust(project: &Project, yes: bool) -> Result<()> {
    let path = project.burrow_dir.join(TRUST_FILE);
    if let Ok(contents) = std::fs::read_to_string(&path) {
        if let Ok(document) = serde_json::from_str::<TrustFile>(&contents) {
            if document.version == TRUST_VERSION && document.approved_hash == project.config_hash {
                return Ok(());
            }
        }
    }

    if yes {
        return write_trust(project, &path);
    }

    if !std::io::stdin().is_terminal() {
        bail!(
            "this project has not been approved and standard input is not a terminal\n\
             help: review burrow.toml, then run `burrow up --yes` (for CI, pass --yes explicitly)"
        );
    }

    eprintln!(
        "Burrow will run commands from {} in {}",
        project.dir.join("burrow.toml").display(),
        project.dir.display()
    );
    eprintln!();
    for (name, service) in &project.config.services {
        if let Some(command) = &service.run {
            eprintln!("  {name}: {command}");
        } else if let Some(reference) = &service.use_spec {
            eprintln!("  {name}: download and run `{reference}` from the Burrow registry");
        }
    }
    if project.dir.join(LOCAL_CONFIG_FILE).exists() {
        eprintln!();
        eprintln!(
            "note: {} is also applied and may change commands",
            LOCAL_CONFIG_FILE
        );
    }
    eprintln!();
    eprintln!("Treat this like a Makefile or package.json script: it can run arbitrary code.");
    eprint!("Approve and start? [y/N] ");
    std::io::stderr().flush().ok();

    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("could not read the approval answer")?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        bail!("not approved; nothing was started");
    }

    write_trust(project, &path)
}

#[derive(Debug, Serialize, Deserialize)]
struct TrustFile {
    version: u32,
    approved_hash: String,
    approved_at: u64,
}

/// Returns true when the current executable configuration is already approved.
pub fn is_trusted(project: &Project) -> bool {
    let path = project.burrow_dir.join(TRUST_FILE);
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(document) = serde_json::from_str::<TrustFile>(&contents) else {
        return false;
    };
    document.version == TRUST_VERSION && document.approved_hash == project.config_hash
}

fn write_trust(project: &Project, path: &Path) -> Result<()> {
    std::fs::create_dir_all(&project.burrow_dir)
        .with_context(|| format!("could not create {}", project.burrow_dir.display()))?;
    let document = TrustFile {
        version: TRUST_VERSION,
        approved_hash: project.config_hash.clone(),
        approved_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
    };
    let temporary = path.with_extension("tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&document).context("could not encode trust approval")?,
    )
    .with_context(|| format!("could not write {}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("could not replace {}", path.display()))?;
    Ok(())
}

/// Downloads and verifies the packages needed by `selected`.
///
/// `burrow up` calls this before detaching so download progress is visible in
/// the terminal. The daemon calls it again; a cached package is a marker check
/// and does not touch the network.
pub async fn warm_packages(
    project: &Project,
    selected: &[String],
    on_progress: &mut dyn FnMut(&str, DownloadProgress),
) -> Result<()> {
    install_packages(project, selected, on_progress, true).await?;
    record_package_refs(project)?;
    Ok(())
}

/// Downloads one registry reference without a project, for `burrow pkg add`.
pub async fn fetch_reference(
    reference: &str,
    on_progress: &mut dyn FnMut(&str, DownloadProgress),
) -> Result<()> {
    let registry = Registry::builtin().map_err(|error| anyhow::anyhow!("{error}"))?;
    let cache = Cache::discover().map_err(|error| anyhow::anyhow!("{error}"))?;
    let resolved = registry
        .resolve(reference)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    cache
        .ensure(&resolved, &mut |progress| on_progress(reference, progress))
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(())
}

/// Installs every package required by `selected` and its dependencies.
async fn install_packages(
    project: &Project,
    selected: &[String],
    on_progress: &mut dyn FnMut(&str, DownloadProgress),
    install: bool,
) -> Result<BTreeMap<String, InstalledPackage>> {
    let startup = project
        .config
        .startup_plan(selected)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let registry = Registry::builtin().map_err(|error| anyhow::anyhow!("{error}"))?;
    let cache = Cache::discover().map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut installed = BTreeMap::new();

    for name in &startup.included {
        let service = &project.config.services[name];
        let Some(reference) = service.use_spec.as_deref() else {
            continue;
        };
        let resolved = registry
            .resolve(reference)
            .map_err(|error| anyhow::anyhow!("service `{name}` uses `{reference}`: {error}"))?;
        let root = if install {
            cache
                .ensure(&resolved, &mut |progress| on_progress(name, progress))
                .await
                .map_err(|error| anyhow::anyhow!("service `{name}`: {error}"))?
        } else {
            // `burrow env` only needs the planned command and URL. It must not
            // download a multi-hundred-megabyte package to print variables.
            cache.package_dir(&resolved.platform.sha256)
        };
        let run = resolved.version.run.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "registry entry `{}` has no run command for Windows",
                resolved.key()
            )
        })?;
        installed.insert(
            name.clone(),
            InstalledPackage {
                key: resolved.key(),
                root,
                url_template: resolved.version.url_template.clone(),
                run,
                stop: resolved.version.stop.clone(),
                init: resolved.version.init.clone(),
                seed: resolved.version.seed.clone(),
                health: resolved.version.health.clone(),
                binaries: resolved.platform.binaries.clone(),
            },
        );
    }
    Ok(installed)
}

/// Records every package reference in the project configuration.
///
/// This is deliberately wider than the current `burrow up` selection: a
/// stopped project still owns its packages, so `burrow pkg gc` must keep them
/// available for an offline restart.
pub fn record_package_refs(project: &Project) -> Result<()> {
    let registry = Registry::builtin().map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut packages = Vec::new();
    for service in project.config.services.values() {
        let Some(reference) = &service.use_spec else {
            continue;
        };
        let resolved = registry
            .resolve(reference)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        packages.push(PackageRef {
            key: resolved.key(),
            sha256: resolved.platform.sha256.clone(),
        });
    }
    if packages.is_empty() {
        return Ok(());
    }
    let cache = Cache::discover().map_err(|error| anyhow::anyhow!("{error}"))?;
    cache
        .record_refs(&project.dir, &project.config.project.name, &packages)
        .map_err(|error| anyhow::anyhow!("{error}"))
}

/// Resolves ports, secrets, packages, templates, and process specifications.
pub async fn resolve(
    project: &Project,
    selected: &[String],
    allocator: &mut PortAllocator,
) -> Result<ResolvedProject> {
    resolve_inner(project, selected, allocator, true).await
}

/// Resolves a project for a shell command, preserving the ports of a running
/// daemon.
///
/// Probing ports would report a different value while the real services are
/// listening, so live assignments are pinned from `.burrow/state.json`. Pass
/// `install = false` to plan package commands without downloading them.
pub async fn resolve_for_shell(
    project: &Project,
    selected: &[String],
    install: bool,
) -> Result<ResolvedProject> {
    let mut allocator = project.ports()?;
    if let Some(state) = crate::control::load_live_state(&project.dir).await? {
        for (name, snapshot) in &state.services {
            allocator.pin(name, snapshot.port);
        }
        if let Some(port) = state.proxy_port {
            allocator.pin(PROXY_PORT_KEY, port);
        }
    }
    let resolved = resolve_inner(project, selected, &mut allocator, install).await?;
    allocator.save()?;
    Ok(resolved)
}

async fn resolve_inner(
    project: &Project,
    selected: &[String],
    allocator: &mut PortAllocator,
    install: bool,
) -> Result<ResolvedProject> {
    let secrets = load_or_create_secrets(project)?;
    let mut config = project.config.clone();
    let startup = config
        .startup_plan(selected)
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let installed = install_packages(
        project,
        selected,
        &mut |name, progress| {
            let percent = progress.downloaded.saturating_mul(100) / progress.total.max(1);
            tracing::info!(service = %name, percent, "preparing package");
        },
        install,
    )
    .await?;
    if install {
        record_package_refs(project)?;
    }

    // Allocate every service, not only the selected ones: a later
    // `burrow up <service>` must reuse the same port.
    let mut ports = BTreeMap::new();
    for (name, service) in &config.services {
        let port = allocator
            .allocate(name, service.port.fixed())
            .with_context(|| format!("could not assign a port to `{name}`"))?;
        ports.insert(name.clone(), port);
    }
    let proxy_port = if startup
        .included
        .iter()
        .any(|name| config.services[name].route.is_some())
    {
        Some(allocate_proxy_port(allocator)?)
    } else {
        None
    };

    let mut variables = BTreeMap::new();
    for (name, port) in &ports {
        let password = secrets.get(name).cloned();
        let url = match installed.get(name) {
            Some(package) => package_url(package, *port, password.as_deref(), project, name)?,
            None => format!("http://127.0.0.1:{port}"),
        };
        variables.insert(
            name.clone(),
            burrow_config::ServiceVars {
                host: "127.0.0.1".to_owned(),
                port: Some(*port),
                url: Some(url),
                password,
            },
        );
    }

    let context = burrow_config::TemplateContext {
        project_name: config.project.name.clone(),
        project_dir: project.dir.clone(),
        services: variables,
    };
    burrow_config::resolve_templates(&mut config, &context)
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let mut services = BTreeMap::new();
    for name in &startup.included {
        let service = &config.services[name];
        let port = ports[name];
        services.insert(
            name.clone(),
            build_service(
                project,
                name,
                service,
                port,
                installed.get(name),
                &secrets,
                install,
            )?,
        );
    }

    Ok(ResolvedProject {
        services,
        startup,
        proxy_port,
    })
}

fn package_vars(
    project: &Project,
    service: &str,
    port: u16,
    password: Option<&str>,
    seed_file: Option<&Path>,
) -> BTreeMap<&'static str, String> {
    let mut vars = BTreeMap::from([
        ("host", "127.0.0.1".to_owned()),
        ("port", port.to_string()),
        ("project_dir", project.dir.to_string_lossy().into_owned()),
        (
            "data_dir",
            project.data_dir(service).to_string_lossy().into_owned(),
        ),
        ("password", password.unwrap_or_default().to_owned()),
    ]);
    if let Some(seed_file) = seed_file {
        vars.insert("seed_file", seed_file.to_string_lossy().into_owned());
    }
    vars
}

fn package_url(
    package: &InstalledPackage,
    port: u16,
    password: Option<&str>,
    project: &Project,
    service: &str,
) -> Result<String> {
    let vars = package_vars(project, service, port, password, None);
    match &package.url_template {
        Some(template) => render_placeholders(template, &vars),
        None => Ok(format!("http://127.0.0.1:{port}")),
    }
}

fn render_placeholders(template: &str, vars: &BTreeMap<&'static str, String>) -> Result<String> {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let after_open = &remaining[start + 2..];
        let Some(end) = after_open.find("}}") else {
            bail!("unclosed `{{{{` in package template `{template}`");
        };
        let key = after_open[..end].trim();
        let value = vars.get(key).ok_or_else(|| {
            anyhow::anyhow!("unknown package template variable `{{{{{key}}}}}` in `{template}`")
        })?;
        output.push_str(value);
        remaining = &after_open[end + 2..];
    }
    if remaining.contains("}}") {
        bail!("unexpected `}}}}` in package template `{template}`");
    }
    output.push_str(remaining);
    Ok(output)
}

fn prepare_command(
    package: &InstalledPackage,
    command: &TemplateCommand,
    vars: &BTreeMap<&'static str, String>,
    password: Option<&str>,
    require_binary: bool,
) -> Result<PreparedCommand> {
    let relative = package.binaries.get(&command.program).ok_or_else(|| {
        anyhow::anyhow!(
            "registry entry `{}` names binary role `{}` but does not define it",
            package.key,
            command.program
        )
    })?;
    let program = package
        .root
        .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    if require_binary && !program.is_file() {
        bail!(
            "package `{}` is missing binary `{}` at {}; run `burrow pkg verify`",
            package.key,
            command.program,
            program.display()
        );
    }

    let mut args = Vec::with_capacity(command.args.len());
    let mut display = vec![command.program.clone()];
    for argument in &command.args {
        let rendered = render_placeholders(argument, vars)?;
        let mut shown = rendered.clone();
        if let Some(password) = password {
            if !password.is_empty() {
                shown = shown.replace(password, "<redacted>");
            }
        }
        display.push(if shown.contains(' ') {
            format!("\"{shown}\"")
        } else {
            shown
        });
        args.push(OsString::from(rendered));
    }

    Ok(PreparedCommand {
        program,
        args,
        display: display.join(" "),
    })
}

fn build_package_plan(
    project: &Project,
    service_name: &str,
    service: &Service,
    port: u16,
    package: &InstalledPackage,
    secrets: &BTreeMap<String, String>,
    require_binaries: bool,
) -> Result<PackagePlan> {
    let password = secrets.get(service_name).map(String::as_str);
    let data_dir = project.data_dir(service_name);
    let mut vars = package_vars(project, service_name, port, password, None);

    let mut init = Vec::with_capacity(package.init.len());
    for step in &package.init {
        init.push(prepare_command(
            package,
            &step.command,
            &vars,
            password,
            require_binaries,
        )?);
    }

    let (seed, seed_file, seed_sha256) =
        if let (Some(command), Some(relative)) = (&package.seed, &service.seed) {
            let path = project.dir.join(relative);
            if !path.is_file() {
                bail!(
                    "service `{service_name}` has seed = \"{}\" but that file does not exist",
                    relative.display()
                );
            }
            vars.insert("seed_file", path.to_string_lossy().into_owned());
            let hash = burrow_data::hash_file(&path)
                .map_err(|error| anyhow::anyhow!("service `{service_name}`: {error}"))?;
            let prepared = prepare_command(package, command, &vars, password, require_binaries)?;
            (Some(prepared), Some(path), Some(hash))
        } else {
            if service.seed.is_some() && package.seed.is_none() {
                tracing::warn!(
                    service = service_name,
                    "package has no seed command; ignoring the configured seed script"
                );
            }
            (None, None, None)
        };

    let mut stop_vars = package_vars(project, service_name, port, password, None);
    stop_vars.insert("seed_file", String::new());
    let stop = package
        .stop
        .as_ref()
        .map(|command| prepare_command(package, command, &stop_vars, password, require_binaries))
        .transpose()?;

    Ok(PackagePlan {
        key: package.key.clone(),
        data_dir,
        init,
        seed,
        seed_file,
        seed_sha256,
        stop,
    })
}

fn package_bin_dirs(package: &InstalledPackage) -> Vec<PathBuf> {
    let mut directories = std::collections::BTreeSet::new();
    for relative in package.binaries.values() {
        let path = Path::new(relative);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                directories.insert(package.root.join(parent));
            }
        }
    }
    directories.into_iter().collect()
}

fn prepend_path(env: &mut BTreeMap<OsString, OsString>, directories: &[PathBuf]) {
    let mut paths = directories.to_vec();
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        env.insert(OsString::from("PATH"), joined);
    }
}

fn build_service(
    project: &Project,
    name: &str,
    service: &Service,
    port: u16,
    package: Option<&InstalledPackage>,
    secrets: &BTreeMap<String, String>,
    require_binaries: bool,
) -> Result<ServicePlan> {
    let cwd = match &service.cwd {
        Some(cwd) => {
            let directory = if cwd.is_absolute() {
                cwd.clone()
            } else {
                project.dir.join(cwd)
            };
            if !directory.is_dir() {
                bail!(
                    "service `{name}` uses cwd `{}` but that directory does not exist",
                    directory.display()
                );
            }
            directory
        }
        None => project.dir.clone(),
    };

    let mut env: BTreeMap<OsString, OsString> = service
        .env
        .iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect();
    let generated = [
        ("PORT", port.to_string()),
        ("HOST", "127.0.0.1".to_owned()),
        ("BURROW_SERVICE", name.to_owned()),
        ("BURROW_PROJECT", project.config.project.name.clone()),
        (
            "BURROW_PROJECT_DIR",
            project.dir.to_string_lossy().into_owned(),
        ),
        (
            "BURROW_DATA_DIR",
            project.data_dir(name).to_string_lossy().into_owned(),
        ),
    ];
    for (key, value) in generated {
        env.insert(OsString::from(key), OsString::from(value));
    }
    env.insert(
        OsString::from("BURROW_SERVICE_PASSWORD"),
        OsString::from(secrets.get(name).cloned().unwrap_or_default()),
    );
    if let Some(package) = package {
        prepend_path(&mut env, &package_bin_dirs(package));
        env.insert(
            OsString::from("BURROW_PACKAGE"),
            OsString::from(package.key.clone()),
        );
        env.insert(
            OsString::from("BURROW_PACKAGE_ROOT"),
            package.root.as_os_str().to_owned(),
        );
    }

    let password = secrets.get(name).map(String::as_str);
    let package_plan = package
        .map(|package| {
            build_package_plan(
                project,
                name,
                service,
                port,
                package,
                secrets,
                require_binaries,
            )
        })
        .transpose()?;

    let (program, args, command_display) = if let Some(command) = &service.run {
        let (program, args) = commandline::split(command)
            .with_context(|| format!("could not parse services.{name}.run"))?;
        (
            PathBuf::from(program),
            args.into_iter().map(OsString::from).collect(),
            command.clone(),
        )
    } else if let Some(package) = package {
        let vars = package_vars(project, name, port, password, None);
        let run = prepare_command(package, &package.run, &vars, password, require_binaries)?;
        (run.program, run.args, run.display)
    } else {
        bail!("service `{name}` has neither `run` nor `use`");
    };

    let url = match package {
        Some(package) => package_url(package, port, password, project, name)?,
        None => format!("http://127.0.0.1:{port}"),
    };

    let health = match &service.health {
        Some(health) => health
            .kind()
            .map(|kind| {
                let check = match kind {
                    HealthKind::Http(path) => HealthCheck::Http { port, path },
                    HealthKind::Tcp => HealthCheck::Tcp { port },
                    HealthKind::Command(command) => HealthCheck::Command {
                        command,
                        cwd: Some(cwd.clone()),
                        env: env.clone(),
                    },
                };
                let options = HealthOptions {
                    timeout: health
                        .timeout()
                        .map_err(|error| anyhow::anyhow!("{error}"))?,
                    interval: health
                        .interval()
                        .map_err(|error| anyhow::anyhow!("{error}"))?,
                    retries: health.retries(),
                };
                Ok::<_, anyhow::Error>(HealthSettings { check, options })
            })
            .transpose()?,
        None => package
            .and_then(|package| package.health.as_ref())
            .map(|health| {
                let check = match &health.http {
                    Some(path) => HealthCheck::Http {
                        port,
                        path: path.clone(),
                    },
                    None => HealthCheck::Tcp { port },
                };
                HealthSettings {
                    check,
                    options: HealthOptions::default(),
                }
            }),
    };

    let mut spec = ProcessSpec::new(name.to_owned(), program)
        .args(args)
        .cwd(cwd.clone())
        .grace_period(
            service
                .grace_period()
                .map_err(|error| anyhow::anyhow!("{error}"))?,
        )
        .log_dir(project.logs_dir());
    spec.env = env.clone();
    if let Some(bytes) = service
        .memory_bytes()
        .map_err(|error| anyhow::anyhow!("{error}"))?
    {
        spec.memory_limit = Some(bytes);
    }
    if let Some(percent) = service
        .cpu_percent()
        .map_err(|error| anyhow::anyhow!("{error}"))?
    {
        spec.cpu_limit_percent = Some(percent);
    }

    Ok(ServicePlan {
        name: name.to_owned(),
        spec,
        port,
        url,
        route: service.route.clone(),
        expose: service.expose,
        health,
        restart: service.restart,
        command_display,
        cwd,
        env,
        package: package_plan,
    })
}

/// Returns the services that must be stopped/started around `name`, in
/// dependency order.
pub fn restart_order(
    config: &Config,
    resolved: &ResolvedProject,
    name: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    if !resolved.services.contains_key(name) {
        bail!("unknown service `{name}`");
    }

    let mut affected: BTreeSet<String> = config.dependents_of(name);
    affected.insert(name.to_owned());
    affected.retain(|candidate| resolved.services.contains_key(candidate));

    let start_order = resolved
        .startup
        .order()
        .into_iter()
        .filter(|candidate| affected.contains(candidate))
        .collect::<Vec<_>>();
    let mut stop_order = start_order.clone();
    stop_order.reverse();
    Ok((stop_order, start_order))
}
