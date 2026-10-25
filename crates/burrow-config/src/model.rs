//! The `burrow.toml` data model and semantic validation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::error::{suggest, ConfigError};

/// Top-level `burrow.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Project metadata.
    pub project: Project,
    /// Services keyed by name.
    #[serde(default)]
    pub services: BTreeMap<String, Service>,
}

/// Project metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// Project name. Lowercase ASCII letters, digits, and hyphens.
    pub name: String,
}

/// One service description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Service {
    /// Registry reference (`postgres@16`) for a managed package.
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    pub use_spec: Option<String>,
    /// Arbitrary command to run. Mutually exclusive with `use`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// Working directory, relative to the project directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Services that must be healthy before this one starts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// Environment variables layered over Burrow's environment.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Fixed port or `"auto"`.
    #[serde(default)]
    pub port: PortConfig,
    /// Health check used before dependents start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<HealthConfig>,
    /// Restart policy.
    #[serde(default)]
    pub restart: RestartPolicy,
    /// Job Object memory cap, for example `"256MB"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    /// Optional hard CPU cap, for example `"50%"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,
    /// How long a graceful stop may take.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grace: Option<String>,
    /// Seed script run once after the first healthy start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<PathBuf>,
    /// Reverse-proxy hostname, for example `shop.localhost`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Bind non-loopback interfaces for this service. Off by default.
    #[serde(default)]
    pub expose: bool,
    /// Phase-2 idle stop duration. The service stops after this much time
    /// without connections and wakes on the next one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_stop: Option<String>,
    /// Phase-2 watch paths, relative to the project directory. The service
    /// restarts when files under these roots change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watch: Vec<String>,
    /// Port the container image listens on inside the container. Only valid
    /// with `use = "container:..."`; defaults to the service's own port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_port: Option<u16>,
}

/// Port selection for a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortConfig {
    /// Allocate a stable free port.
    #[default]
    Auto,
    /// Use this exact port.
    Fixed(u16),
}

impl PortConfig {
    /// Returns the fixed port, if any.
    pub fn fixed(self) -> Option<u16> {
        match self {
            Self::Auto => None,
            Self::Fixed(port) => Some(port),
        }
    }
}

impl Serialize for PortConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Fixed(port) => serializer.serialize_u16(*port),
        }
    }
}

impl<'de> Deserialize<'de> for PortConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = PortConfig;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("`auto` or a port number from 1 to 65535")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value.eq_ignore_ascii_case("auto") {
                    Ok(PortConfig::Auto)
                } else {
                    Err(E::custom("expected `auto` or a port number"))
                }
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                let port = u16::try_from(value)
                    .ok()
                    .filter(|port| *port > 0)
                    .ok_or_else(|| E::custom("port must be between 1 and 65535"))?;
                Ok(PortConfig::Fixed(port))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                if value < 1 {
                    return Err(E::custom("port must be between 1 and 65535"));
                }
                self.visit_u64(value as u64)
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

impl JsonSchema for PortConfig {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Port".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({
            "oneOf": [
                { "type": "string", "const": "auto" },
                { "type": "integer", "minimum": 1, "maximum": 65535 }
            ]
        })
    }
}

/// Health check configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HealthConfig {
    /// HTTP path, for example `/health`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<String>,
    /// Wait for a TCP connection on the service port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp: Option<bool>,
    /// Run a command; success means exit code 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
    /// Total time to keep trying, for example `"60s"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
    /// Delay between attempts, for example `"500ms"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,
    /// Maximum number of attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
}

/// The active check selected in a [`HealthConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthKind {
    /// HTTP GET against a path on the service's port.
    Http(String),
    /// TCP connect to the service's port.
    Tcp,
    /// Run a command and require exit code 0.
    Command(String),
}

impl HealthConfig {
    /// Returns the active check, or `None` when no check is configured.
    pub fn kind(&self) -> Option<HealthKind> {
        if let Some(path) = &self.http {
            Some(HealthKind::Http(path.clone()))
        } else if self.tcp == Some(true) {
            Some(HealthKind::Tcp)
        } else {
            self.cmd.clone().map(HealthKind::Command)
        }
    }

    /// Total time to keep trying.
    pub fn timeout(&self) -> Result<Duration, ConfigError> {
        parse_optional_duration(self.timeout.as_deref(), "60s", "health.timeout")
    }

    /// Delay between attempts.
    pub fn interval(&self) -> Result<Duration, ConfigError> {
        parse_optional_duration(self.interval.as_deref(), "500ms", "health.interval")
    }

    /// Maximum number of attempts.
    pub fn retries(&self) -> u32 {
        self.retries.unwrap_or(120)
    }
}

/// Restart behaviour after an unexpected exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    /// Never restart.
    Never,
    /// Restart only after a non-zero exit.
    #[default]
    OnFailure,
    /// Always restart.
    Always,
}

impl RestartPolicy {
    /// Whether a process with `success` should be restarted.
    pub fn should_restart(self, success: bool) -> bool {
        match self {
            Self::Never => false,
            Self::OnFailure => !success,
            Self::Always => true,
        }
    }
}

impl Config {
    /// Validates names, references, limits, and the dependency graph.
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_identifier("project.name", &self.project.name)?;

        if self.services.is_empty() {
            return Err(ConfigError::invalid(
                "services",
                "define at least one service, for example `[services.api]`",
            ));
        }

        let mut routes: BTreeMap<&str, &str> = BTreeMap::new();

        for (name, service) in &self.services {
            let path = format!("services.{name}");
            validate_identifier(&path, name)?;

            match (&service.use_spec, &service.run) {
                (Some(_), Some(_)) => {
                    return Err(ConfigError::invalid(
                        &path,
                        "`use` and `run` are mutually exclusive; remove one",
                    ))
                }
                (None, None) => {
                    return Err(ConfigError::invalid(
                        &path,
                        "add either `use = \"postgres@16\"` or `run = \"...\"`",
                    ))
                }
                _ => {}
            }

            if let Some(reference) = &service.use_spec {
                validate_registry_reference(&format!("{path}.use"), reference)?;
            }
            if let Some(port) = service.container_port {
                if port == 0 {
                    return Err(ConfigError::invalid(
                        format!("{path}.container_port"),
                        "port must be between 1 and 65535",
                    ));
                }
                if service.container_image().is_none() {
                    return Err(ConfigError::invalid(
                        format!("{path}.container_port"),
                        "`container_port` is only valid with `use = \"container:IMAGE\"`",
                    ));
                }
            }
            if service.seed.is_some() && service.container_image().is_some() {
                return Err(ConfigError::invalid(
                    format!("{path}.seed"),
                    "container services do not support seed scripts; \
                     use a native registry package for seeded data",
                ));
            }
            for entry in &service.watch {
                validate_relative_entry(&format!("{path}.watch"), Path::new(entry), "watch paths")?;
            }
            if let Some(command) = &service.run {
                if command.trim().is_empty() {
                    return Err(ConfigError::invalid(
                        format!("{path}.run"),
                        "command must not be empty",
                    ));
                }
            }

            service.memory_bytes()?;
            service.cpu_percent()?;
            service.grace_period()?;
            if let Some(idle_stop) = &service.idle_stop {
                let duration = parse_duration(idle_stop, &format!("{path}.idle_stop"))?;
                if duration.is_zero() {
                    return Err(ConfigError::invalid(
                        format!("{path}.idle_stop"),
                        "duration must be greater than zero",
                    ));
                }
            }
            if let Some(route) = &service.route {
                validate_route(&format!("{path}.route"), route)?;
                if let Some(existing) = routes.insert(route, name) {
                    return Err(ConfigError::invalid(
                        format!("{path}.route"),
                        format!("route `{route}` is already used by service `{existing}`"),
                    ));
                }
            }
            if let Some(seed) = &service.seed {
                validate_relative_script(&format!("{path}.seed"), seed)?;
            }
            for (index, dependency) in service.needs.iter().enumerate() {
                if dependency == name {
                    return Err(ConfigError::invalid(
                        format!("{path}.needs[{index}]"),
                        "a service cannot depend on itself",
                    ));
                }
                if !self.services.contains_key(dependency) {
                    let suggestion = suggest(dependency, self.services.keys())
                        .map(|candidate| format!("; did you mean `{candidate}`?"))
                        .unwrap_or_default();
                    return Err(ConfigError::invalid(
                        format!("{path}.needs[{index}]"),
                        format!("unknown service `{dependency}`{suggestion}"),
                    ));
                }
            }
            if let Some(health) = &service.health {
                validate_health(&format!("{path}.health"), health)?;
            }
        }

        self.startup_plan(&[])?;
        Ok(())
    }

    /// Builds a deterministic startup plan, optionally limited to `selected`
    /// services (plus their transitive dependencies).
    pub fn startup_plan(&self, selected: &[String]) -> Result<crate::StartupPlan, ConfigError> {
        crate::graph::build(self, selected)
    }

    /// Returns the enabled services: all services, or `selected` plus deps.
    pub fn selected_services(&self, selected: &[String]) -> Result<BTreeSet<String>, ConfigError> {
        let mut included: BTreeSet<String> = if selected.is_empty() {
            self.services.keys().cloned().collect()
        } else {
            selected.iter().cloned().collect()
        };

        for name in selected {
            if !self.services.contains_key(name) {
                let suggestion = suggest(name, self.services.keys())
                    .map(|candidate| format!("; did you mean `{candidate}`?"))
                    .unwrap_or_default();
                return Err(ConfigError::invalid(
                    "services",
                    format!("unknown service `{name}`{suggestion}"),
                ));
            }
        }

        let mut queue: Vec<String> = included.iter().cloned().collect();
        while let Some(name) = queue.pop() {
            let service = &self.services[&name];
            for dependency in &service.needs {
                if included.insert(dependency.clone()) {
                    queue.push(dependency.clone());
                }
            }
        }

        Ok(included)
    }

    /// Returns the transitive dependents of `name` (not including `name`).
    pub fn dependents_of(&self, name: &str) -> BTreeSet<String> {
        let mut dependents = BTreeSet::new();
        let mut queue = vec![name.to_owned()];
        while let Some(current) = queue.pop() {
            for (candidate, service) in &self.services {
                if service.needs.iter().any(|need| need == &current)
                    && dependents.insert(candidate.clone())
                {
                    queue.push(candidate.clone());
                }
            }
        }
        dependents
    }

    /// Hash of the executable content that requires user approval.
    ///
    /// Ports, memory, health checks, and other operational settings are
    /// excluded: changing them does not require re-approval.
    pub fn executable_hash(&self) -> String {
        let services: BTreeMap<&String, serde_json::Value> = self
            .services
            .iter()
            .map(|(name, service)| {
                (
                    name,
                    serde_json::json!({
                        "use": &service.use_spec,
                        "run": &service.run,
                        "cwd": &service.cwd,
                    }),
                )
            })
            .collect();

        let canonical = serde_json::to_vec(&serde_json::json!({
            "project": self.project.name,
            "services": services,
        }))
        .expect("configuration values are JSON-serializable");

        let digest = Sha256::digest(canonical);
        hex::encode(digest)
    }
}

impl Service {
    /// Parses the memory cap, if configured.
    pub fn memory_bytes(&self) -> Result<Option<u64>, ConfigError> {
        self.memory
            .as_deref()
            .map(|raw| {
                parse_byte_size(raw).map_err(|message| {
                    ConfigError::invalid("services.memory", format!("{message}: `{raw}`"))
                })
            })
            .transpose()
    }

    /// Parses the CPU cap, if configured.
    pub fn cpu_percent(&self) -> Result<Option<u8>, ConfigError> {
        self.cpu
            .as_deref()
            .map(|raw| {
                parse_cpu_percent(raw).map_err(|message| {
                    ConfigError::invalid("services.cpu", format!("{message}: `{raw}`"))
                })
            })
            .transpose()
    }

    /// Grace period for shutdown.
    pub fn grace_period(&self) -> Result<Duration, ConfigError> {
        parse_optional_duration(self.grace.as_deref(), "10s", "services.grace")
    }

    /// Returns the health check kind, if any.
    pub fn health_kind(&self) -> Option<HealthKind> {
        self.health.as_ref().and_then(HealthConfig::kind)
    }

    /// Whether this service comes from the registry.
    pub fn is_registry(&self) -> bool {
        self.use_spec.is_some() && self.container_image().is_none()
    }

    /// The container image reference when `use` is `container:IMAGE`.
    pub fn container_image(&self) -> Option<&str> {
        self.use_spec
            .as_deref()
            .and_then(|reference| reference.strip_prefix("container:"))
    }
}

/// Parses a byte size such as `256MB` or `1 GiB`.
pub fn parse_byte_size(raw: &str) -> Result<u64, String> {
    let normalized = raw.trim().replace(' ', "");
    let split = normalized
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(normalized.len());
    let (number, unit) = normalized.split_at(split);
    if number.is_empty() {
        return Err("expected a size such as `256MB`".to_owned());
    }
    let value: u64 = number.parse().map_err(|_| "size is too large".to_owned())?;
    if value == 0 {
        return Err("size must be greater than zero".to_owned());
    }

    let multiplier: u64 = match unit.to_ascii_uppercase().as_str() {
        "B" => 1,
        "K" | "KB" | "KIB" => 1024,
        "M" | "MB" | "MIB" => 1024 * 1024,
        "G" | "GB" | "GIB" => 1024 * 1024 * 1024,
        "T" | "TB" | "TIB" => 1024 * 1024 * 1024 * 1024,
        "" => return Err("add a unit, for example `256MB`".to_owned()),
        other => return Err(format!("unknown unit `{other}`; use B, KB, MB, GB, or TB")),
    };

    value
        .checked_mul(multiplier)
        .ok_or_else(|| "size is too large".to_owned())
}

/// Parses a CPU percentage such as `50%`.
pub fn parse_cpu_percent(raw: &str) -> Result<u8, String> {
    let trimmed = raw.trim();
    let number = trimmed
        .strip_suffix('%')
        .ok_or_else(|| "CPU limit must include `%`, for example `50%`".to_owned())?
        .trim();
    let percent: u8 = number
        .parse()
        .map_err(|_| "CPU limit must be a whole number from 1 to 100".to_owned())?;
    if !(1..=100).contains(&percent) {
        return Err("CPU limit must be between 1% and 100%".to_owned());
    }
    Ok(percent)
}

/// Parses a human duration such as `10m` or `500ms`.
pub fn parse_duration(raw: &str, path: &str) -> Result<Duration, ConfigError> {
    humantime::parse_duration(raw.trim())
        .map_err(|error| ConfigError::invalid(path, format!("invalid duration `{raw}`: {error}")))
}

fn parse_optional_duration(
    raw: Option<&str>,
    default: &str,
    path: &str,
) -> Result<Duration, ConfigError> {
    parse_duration(raw.unwrap_or(default), path)
}

fn validate_identifier(path: &str, value: &str) -> Result<(), ConfigError> {
    if value.is_empty() {
        return Err(ConfigError::invalid(path, "must not be empty"));
    }
    if value.len() > 63 {
        return Err(ConfigError::invalid(path, "must be 63 characters or fewer"));
    }
    if value.starts_with('-') || value.ends_with('-') {
        return Err(ConfigError::invalid(
            path,
            "must not start or end with a hyphen",
        ));
    }
    if !value.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    }) {
        return Err(ConfigError::invalid(
            path,
            "use lowercase ASCII letters, digits, and hyphens only",
        ));
    }
    if is_reserved_windows_name(value) {
        return Err(ConfigError::invalid(
            path,
            "must not be a reserved Windows device name such as `con`, `nul`, or `com1`",
        ));
    }
    Ok(())
}

/// Returns true when `value` is a reserved Windows device name.
///
/// Windows reserves `con`, `prn`, `aux`, `nul`, `com1`-`com9`, and
/// `lpt1`-`lpt9` as device names, with or without an extension. Configuration
/// names become directory names under `.burrow/data/`, so they must not use
/// one.
pub fn is_reserved_windows_name(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or(value)
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn validate_registry_reference(path: &str, reference: &str) -> Result<(), ConfigError> {
    if let Some(image) = reference.strip_prefix("container:") {
        if image.trim().is_empty() || image.chars().any(char::is_whitespace) {
            return Err(ConfigError::invalid(
                path,
                "`container:` needs an image reference, for example `container:redis:7`",
            ));
        }
        return Ok(());
    }

    let (name, version) = reference.split_once('@').ok_or_else(|| {
        ConfigError::invalid(
            path,
            "expected `name@version` or `container:image`, for example `postgres@16`",
        )
    })?;
    validate_identifier(path, name)?;
    if version.trim().is_empty()
        || version.chars().any(char::is_whitespace)
        || version.contains('@')
    {
        return Err(ConfigError::invalid(
            path,
            "version must not be empty or contain whitespace",
        ));
    }
    Ok(())
}

fn validate_route(path: &str, route: &str) -> Result<(), ConfigError> {
    if !route.contains('.') {
        return Err(ConfigError::invalid(
            path,
            "route must be a hostname containing a dot, for example `shop.localhost`",
        ));
    }
    if route.starts_with('.') || route.ends_with('.') {
        return Err(ConfigError::invalid(
            path,
            "route must not start or end with a dot",
        ));
    }
    if !route.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '.'
    }) {
        return Err(ConfigError::invalid(
            path,
            "route may use lowercase letters, digits, hyphens, and dots only",
        ));
    }
    Ok(())
}

fn validate_relative_script(path: &str, script: &Path) -> Result<(), ConfigError> {
    validate_relative_entry(path, script, "seed scripts")
}

fn validate_relative_entry(path: &str, value: &Path, what: &str) -> Result<(), ConfigError> {
    if value.is_absolute() {
        return Err(ConfigError::invalid(
            path,
            format!("{what} must be relative to the project directory"),
        ));
    }
    if value
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(ConfigError::invalid(
            path,
            format!("{what} must not traverse outside the project directory"),
        ));
    }
    Ok(())
}

fn validate_health(path: &str, health: &HealthConfig) -> Result<(), ConfigError> {
    let checks = usize::from(health.http.is_some())
        + usize::from(health.tcp.is_some())
        + usize::from(health.cmd.is_some());
    if checks != 1 {
        return Err(ConfigError::invalid(
            path,
            "set exactly one of `http`, `tcp`, or `cmd`",
        ));
    }
    if health.tcp == Some(false) {
        return Err(ConfigError::invalid(
            format!("{path}.tcp"),
            "`tcp` must be `true`; remove the key to disable the check",
        ));
    }
    if let Some(http) = &health.http {
        if !http.starts_with('/') {
            return Err(ConfigError::invalid(
                format!("{path}.http"),
                "HTTP health path must start with `/`",
            ));
        }
    }
    if let Some(command) = &health.cmd {
        if command.trim().is_empty() {
            return Err(ConfigError::invalid(
                format!("{path}.cmd"),
                "health command must not be empty",
            ));
        }
    }
    let timeout = health.timeout()?;
    let interval = health.interval()?;
    if timeout.is_zero() || interval.is_zero() {
        return Err(ConfigError::invalid(
            path,
            "timeout and interval must be greater than zero",
        ));
    }
    if health.retries() == 0 {
        return Err(ConfigError::invalid(
            format!("{path}.retries"),
            "retries must be at least 1",
        ));
    }
    Ok(())
}

pub(crate) fn schema() -> String {
    serde_json::to_string_pretty(&schemars::schema_for!(Config))
        .expect("the generated schema is JSON-serializable")
}
