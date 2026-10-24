//! Registry parsing, validation, and version resolution.
//!
//! A registry document is one TOML file per service. The built-in entries are
//! embedded at compile time, so a single `burrow.exe` can install packages
//! without a side-car data directory. Every entry carries the exact upstream
//! URL, size, and SHA-256 for its artifacts.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{Error, Result};

/// Registry schema version understood by this build.
pub const REGISTRY_SCHEMA: u32 = 1;
/// Platform key used for Windows x86-64 artifacts.
pub const WINDOWS_X86_64: &str = "windows-x86_64";

/// Archive container formats supported by the hardened extractor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    /// A ZIP archive.
    Zip,
    /// A gzip-compressed tar archive.
    TarGz,
    /// An uncompressed tar archive.
    Tar,
    /// A single binary copied into place.
    Raw,
}

impl ArchiveKind {
    /// File extension used in the cache.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::TarGz => "tar.gz",
            Self::Tar => "tar",
            Self::Raw => "bin",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "zip" => Some(Self::Zip),
            "tar.gz" | "tgz" => Some(Self::TarGz),
            "tar" => Some(Self::Tar),
            "raw" => Some(Self::Raw),
            _ => None,
        }
    }
}

impl fmt::Display for ArchiveKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Zip => "zip",
            Self::TarGz => "tar.gz",
            Self::Tar => "tar",
            Self::Raw => "raw",
        })
    }
}

/// When an initialization command is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InitWhen {
    /// Run only when the service's data directory is missing or empty.
    #[default]
    DataEmpty,
}

/// A command template stored in the registry.
///
/// `program` names a role in the platform's `binaries` map. Arguments may use
/// `{{port}}`, `{{host}}`, `{{data_dir}}`, `{{project_dir}}`, `{{password}}`,
/// and `{{seed_file}}`; the CLI substitutes them without a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateCommand {
    /// Binary role, for example `postgres`.
    pub program: String,
    /// Arguments with optional placeholders.
    pub args: Vec<String>,
}

/// One required initialization step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitStep {
    /// Command to run.
    pub command: TemplateCommand,
    /// Gate for running it.
    pub when: InitWhen,
}

/// The health check a managed package advertises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageHealth {
    /// Wait for a TCP connection on the allocated port.
    pub tcp: bool,
    /// HTTP path on the allocated port.
    pub http: Option<String>,
}

/// A per-platform artifact and its binaries.
#[derive(Debug, Clone)]
pub struct Platform {
    /// HTTPS URL of the artifact.
    pub url: String,
    /// Lowercase SHA-256 of the complete artifact.
    pub sha256: String,
    /// Exact size in bytes.
    pub size: u64,
    /// Container format.
    pub archive: ArchiveKind,
    /// Binary roles to relative paths inside the extracted root.
    pub binaries: BTreeMap<String, String>,
}

/// Everything the runtime needs for one version of a package.
#[derive(Debug, Clone)]
pub struct PackageVersion {
    /// Version key exactly as written in the registry.
    pub version: String,
    /// Artifact metadata keyed by platform.
    pub platforms: BTreeMap<String, Platform>,
    /// Foreground command that starts the service.
    pub run: Option<TemplateCommand>,
    /// Optional graceful stop command.
    pub stop: Option<TemplateCommand>,
    /// Required initialization steps, in order.
    pub init: Vec<InitStep>,
    /// Optional database seed command; `{{seed_file}}` is replaced by the
    /// project's `seed` path.
    pub seed: Option<TemplateCommand>,
    /// URL template for `{{service.url}}` and `{{service.password}}` users.
    pub url_template: Option<String>,
    /// Health check advertised by the package.
    pub health: Option<PackageHealth>,
    /// Suggested port (Burrow still allocates by default).
    pub default_port: Option<u16>,
}

/// One registry entry, which may contain several versions.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Package name, for example `postgres`.
    pub name: String,
    /// One-line description.
    pub description: String,
    /// Upstream license identifier.
    pub license: String,
    /// Upstream project page.
    pub upstream: String,
    /// Page where the Windows build was obtained.
    pub source: String,
    /// Set when the package has no trustworthy native Windows build.
    pub unsupported_native: Option<String>,
    /// Versions keyed exactly as written.
    pub versions: BTreeMap<String, PackageVersion>,
}

impl Entry {
    /// Returns the highest version, or an error when none exist.
    pub fn latest_version(&self) -> Option<&PackageVersion> {
        self.versions
            .values()
            .max_by(|left, right| compare_versions(&left.version, &right.version))
    }
}

/// A fully resolved package reference for the current platform.
#[derive(Debug, Clone, Copy)]
pub struct Resolved<'a> {
    /// Registry entry.
    pub entry: &'a Entry,
    /// Selected version.
    pub version: &'a PackageVersion,
    /// Windows x86-64 artifact.
    pub platform: &'a Platform,
}

impl Resolved<'_> {
    /// Canonical `name@version` key used by the cache and reference tracking.
    pub fn key(&self) -> String {
        format!("{}@{}", self.entry.name, self.version.version)
    }

    /// Absolute path of a binary role inside an installed root.
    pub fn binary_path(&self, root: &std::path::Path, role: &str) -> Option<PathBuf> {
        self.platform
            .binaries
            .get(role)
            .map(|relative| root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR)))
    }
}

/// All built-in registry entries plus any environment-provided overrides.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    entries: BTreeMap<String, Entry>,
}

impl Registry {
    /// Parses the entries embedded in this binary.
    pub fn builtin() -> Result<Self> {
        let mut registry = Self::default();
        for (label, text) in crate::builtin::ENTRIES {
            registry.add_toml(text, label)?;
        }
        Ok(registry)
    }

    /// Parses a registry document and inserts it.
    pub fn add_toml(&mut self, text: &str, label: &str) -> Result<()> {
        let entry = parse_entry(text, label)?;
        if self.entries.contains_key(&entry.name) {
            return Err(Error::InvalidRegistry {
                name: entry.name.clone(),
                message: format!("a registry entry with this name is already loaded ({label})"),
            });
        }
        self.entries.insert(entry.name.clone(), entry);
        Ok(())
    }

    /// Loads every `*.toml` file in `directory`, sorted by file name.
    pub fn load_directory(&mut self, directory: &std::path::Path) -> Result<()> {
        let mut files = Vec::new();
        let entries = std::fs::read_dir(directory).map_err(|error| Error::io(directory, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| Error::io(directory, error))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("toml") {
                files.push(path);
            }
        }
        files.sort();
        for path in files {
            let text = std::fs::read_to_string(&path).map_err(|error| Error::io(&path, error))?;
            self.add_toml(&text, &path.display().to_string())?;
        }
        Ok(())
    }

    /// Returns the entries in name order.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    /// Returns one entry by name.
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.get(name)
    }

    /// Resolves `name@version` (or `name` for the latest version) on Windows
    /// x86-64.
    pub fn resolve(&self, reference: &str) -> Result<Resolved<'_>> {
        let (name, requested) = match reference.split_once('@') {
            Some((name, version)) => (name.trim(), version.trim()),
            None => (reference.trim(), ""),
        };
        let entry = self
            .entries
            .get(name)
            .ok_or_else(|| Error::UnknownPackage {
                name: name.to_owned(),
                available: self
                    .entries
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
            })?;

        if let Some(reason) = &entry.unsupported_native {
            return Err(Error::UnsupportedNative {
                name: entry.name.clone(),
                reason: reason.clone(),
            });
        }
        if entry.versions.is_empty() {
            return Err(Error::UnknownVersion {
                name: entry.name.clone(),
                requested: requested.to_owned(),
                available: "none".to_owned(),
            });
        }

        let requested = if requested.is_empty() || requested == "latest" {
            None
        } else {
            Some(requested)
        };
        let version = select_version(entry, requested)?;
        let platform =
            version
                .platforms
                .get(WINDOWS_X86_64)
                .ok_or_else(|| Error::InvalidRegistry {
                    name: entry.name.clone(),
                    message: format!(
                        "version {} has no `{WINDOWS_X86_64}` artifact",
                        version.version
                    ),
                })?;

        Ok(Resolved {
            entry,
            version,
            platform,
        })
    }
}

fn select_version<'a>(entry: &'a Entry, requested: Option<&str>) -> Result<&'a PackageVersion> {
    let Some(requested) = requested else {
        return entry.latest_version().ok_or_else(|| Error::UnknownVersion {
            name: entry.name.clone(),
            requested: "latest".to_owned(),
            available: "none".to_owned(),
        });
    };

    if let Some(exact) = entry.versions.get(requested) {
        return Ok(exact);
    }

    let wanted = parse_version(requested).ok_or_else(|| Error::UnknownVersion {
        name: entry.name.clone(),
        requested: requested.to_owned(),
        available: available_versions(entry),
    })?;

    entry
        .versions
        .values()
        .filter(|version| {
            parse_version(&version.version).is_some_and(|candidate| candidate.starts_with(&wanted))
        })
        .max_by(|left, right| compare_versions(&left.version, &right.version))
        .ok_or_else(|| Error::UnknownVersion {
            name: entry.name.clone(),
            requested: requested.to_owned(),
            available: available_versions(entry),
        })
}

fn available_versions(entry: &Entry) -> String {
    let mut versions: Vec<_> = entry.versions.keys().map(String::as_str).collect();
    versions.sort_by(|left, right| compare_versions(left, right));
    versions.join(", ")
}

fn parse_version(value: &str) -> Option<Vec<u64>> {
    value
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect()
}

fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    match (parse_version(left), parse_version(right)) {
        (Some(left), Some(right)) => left.cmp(&right).then_with(|| left.len().cmp(&right.len())),
        _ => left.cmp(right),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    schema: u32,
    name: String,
    description: String,
    license: String,
    upstream: String,
    source: String,
    #[serde(default)]
    unsupported_native: Option<String>,
    #[serde(default)]
    versions: BTreeMap<String, RawVersion>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVersion {
    #[serde(default)]
    platforms: BTreeMap<String, RawPlatform>,
    #[serde(default)]
    run: Option<RawCommand>,
    #[serde(default)]
    stop: Option<RawCommand>,
    #[serde(default)]
    init: Vec<RawInit>,
    #[serde(default)]
    seed: Option<RawCommand>,
    #[serde(default)]
    url_template: Option<String>,
    #[serde(default)]
    health: Option<RawHealth>,
    #[serde(default)]
    default_port: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlatform {
    url: String,
    sha256: String,
    size: u64,
    archive: String,
    #[serde(default)]
    binaries: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    program: String,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInit {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    when: RawInitWhen,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RawInitWhen {
    #[default]
    DataEmpty,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHealth {
    #[serde(default)]
    tcp: bool,
    #[serde(default)]
    http: Option<String>,
}

fn parse_entry(text: &str, label: &str) -> Result<Entry> {
    let raw: RawEntry = toml::from_str(text).map_err(|error| Error::InvalidRegistry {
        name: label.to_owned(),
        message: error.to_string(),
    })?;

    let fail = |message: String| Error::InvalidRegistry {
        name: raw.name.clone(),
        message,
    };

    if raw.schema != REGISTRY_SCHEMA {
        return Err(fail(format!(
            "unsupported schema {}; this build understands {REGISTRY_SCHEMA}",
            raw.schema
        )));
    }
    validate_identifier(&raw.name).map_err(|reason| fail(format!("invalid name: {reason}")))?;
    if raw.description.trim().is_empty() {
        return Err(fail("description must not be empty".to_owned()));
    }
    if raw.license.trim().is_empty() {
        return Err(fail("license must not be empty".to_owned()));
    }
    validate_https(&raw.upstream).map_err(|reason| fail(format!("upstream: {reason}")))?;
    validate_https(&raw.source).map_err(|reason| fail(format!("source: {reason}")))?;

    let mut versions = BTreeMap::new();
    for (key, version) in raw.versions {
        if parse_version(&key).is_none() {
            return Err(fail(format!(
                "version key `{key}` must be numeric components separated by dots, for example `16.15`"
            )));
        }
        let version = convert_version(&raw.name, &key, version)?;
        versions.insert(key, version);
    }

    if raw.unsupported_native.is_none() && versions.is_empty() {
        return Err(fail(
            "define at least one version or set `unsupported_native` with a reason".to_owned(),
        ));
    }

    Ok(Entry {
        name: raw.name,
        description: raw.description,
        license: raw.license,
        upstream: raw.upstream,
        source: raw.source,
        unsupported_native: raw.unsupported_native,
        versions,
    })
}

fn convert_version(name: &str, key: &str, raw: RawVersion) -> Result<PackageVersion> {
    let fail = |message: String| Error::InvalidRegistry {
        name: name.to_owned(),
        message: format!("version {key}: {message}"),
    };

    let mut platforms = BTreeMap::new();
    for (platform_key, platform) in raw.platforms {
        if !matches!(platform_key.as_str(), WINDOWS_X86_64) {
            return Err(fail(format!(
                "unsupported platform `{platform_key}`; this build only ships `{WINDOWS_X86_64}`"
            )));
        }
        validate_https(&platform.url).map_err(|reason| fail(format!("url: {reason}")))?;
        if platform.sha256.len() != 64
            || !platform
                .sha256
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(fail(format!(
                "sha256 must be 64 hexadecimal characters, got `{}`",
                platform.sha256
            )));
        }
        if platform.size == 0 {
            return Err(fail("size must be greater than zero".to_owned()));
        }
        let archive = ArchiveKind::parse(&platform.archive).ok_or_else(|| {
            fail(format!(
                "unknown archive type `{}`; use zip, tar.gz, tar, or raw",
                platform.archive
            ))
        })?;
        if platform.binaries.is_empty() {
            return Err(fail("binaries must not be empty".to_owned()));
        }
        for (role, relative) in &platform.binaries {
            validate_identifier(role)
                .map_err(|reason| fail(format!("binary role `{role}`: {reason}")))?;
            validate_relative(relative)
                .map_err(|reason| fail(format!("binary `{role}` path `{relative}`: {reason}")))?;
        }

        platforms.insert(
            platform_key,
            Platform {
                url: platform.url,
                sha256: platform.sha256.to_ascii_lowercase(),
                size: platform.size,
                archive,
                binaries: platform.binaries,
            },
        );
    }

    let convert_command = |command: RawCommand| TemplateCommand {
        program: command.program,
        args: command.args,
    };
    let run = raw.run.map(convert_command);
    let stop = raw.stop.map(convert_command);
    let seed = raw.seed.map(convert_command);
    let init: Vec<InitStep> = raw
        .init
        .into_iter()
        .map(|step| InitStep {
            command: TemplateCommand {
                program: step.program,
                args: step.args,
            },
            when: match step.when {
                RawInitWhen::DataEmpty => InitWhen::DataEmpty,
            },
        })
        .collect();

    let platform = platforms.get(WINDOWS_X86_64);
    for command in run
        .iter()
        .chain(stop.iter())
        .chain(seed.iter())
        .chain(init.iter().map(|step| &step.command))
    {
        if let Some(platform) = platform {
            if !platform.binaries.contains_key(&command.program) {
                return Err(fail(format!(
                    "command refers to binary role `{}` which is missing from `binaries`",
                    command.program
                )));
            }
        }
    }

    if let Some(health) = &raw.health {
        if !health.tcp && health.http.is_none() {
            return Err(fail(
                "health must set `tcp = true` or `http = \"/path\"`".to_owned(),
            ));
        }
    }

    Ok(PackageVersion {
        version: key.to_owned(),
        platforms,
        run,
        stop,
        init,
        seed,
        url_template: raw.url_template,
        health: raw.health.map(|health| PackageHealth {
            tcp: health.tcp,
            http: health.http,
        }),
        default_port: raw.default_port,
    })
}

fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err("must not be empty".to_owned());
    }
    if !value.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '_'
    }) {
        return Err("use lowercase ASCII letters, digits, hyphens, and underscores".to_owned());
    }
    Ok(())
}

fn validate_https(value: &str) -> std::result::Result<(), String> {
    if !value.starts_with("https://") {
        return Err(format!("`{value}` must use https://"));
    }
    Ok(())
}

/// Validates a relative path from a registry entry and returns its components.
pub(crate) fn validate_relative(value: &str) -> std::result::Result<PathBuf, String> {
    if value.is_empty() {
        return Err("must not be empty".to_owned());
    }
    if value.contains('\\') {
        return Err("use forward slashes".to_owned());
    }
    let path = PathBuf::from(value);
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.contains(':') {
                    return Err("must not contain `:`".to_owned());
                }
                if part.ends_with('.') || part.ends_with(' ') {
                    return Err(format!("`{part}` must not end with a dot or space"));
                }
                if is_reserved_windows_name(&part) {
                    return Err(format!("`{part}` is a reserved Windows name"));
                }
            }
            _ => return Err("must be a relative path without `..`".to_owned()),
        }
    }
    Ok(path)
}

fn is_reserved_windows_name(value: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::{ArchiveKind, Registry, WINDOWS_X86_64};

    const SAMPLE: &str = r#"
schema = 1
name = "demo"
description = "Demo service"
license = "MIT"
upstream = "https://example.com/demo"
source = "https://example.com/demo/downloads"

[versions."1.2"]
url_template = "tcp://{{host}}:{{port}}"
health = { tcp = true }
run = { program = "server", args = ["--port", "{{port}}"] }
init = [{ program = "init", args = ["--data", "{{data_dir}}"] }]
stop = { program = "stop", args = [] }

[versions."1.2".platforms.windows-x86_64]
url = "https://example.com/demo-1.2.zip"
sha256 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
size = 1234
archive = "zip"

[versions."1.2".platforms.windows-x86_64.binaries]
server = "bin/server.exe"
init = "bin/init.exe"
stop = "bin/stop.exe"

[versions."1.10"]
url_template = "tcp://{{host}}:{{port}}"
run = { program = "server", args = [] }

[versions."1.10".platforms.windows-x86_64]
url = "https://example.com/demo-1.10.zip"
sha256 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
size = 4321
archive = "zip"

[versions."1.10".platforms.windows-x86_64.binaries]
server = "bin/server.exe"
"#;

    fn registry() -> Registry {
        let mut registry = Registry::default();
        registry.add_toml(SAMPLE, "sample").expect("parse sample");
        registry
    }

    #[test]
    fn version_prefixes_select_the_highest_match() {
        let registry = registry();
        assert_eq!(
            registry.resolve("demo@1").expect("major").version.version,
            "1.10"
        );
        assert_eq!(
            registry.resolve("demo@1.2").expect("minor").version.version,
            "1.2"
        );
        assert_eq!(
            registry.resolve("demo").expect("latest").version.version,
            "1.10"
        );
        assert_eq!(
            registry
                .resolve("demo@1.10")
                .expect("exact")
                .version
                .version,
            "1.10"
        );
    }

    #[test]
    fn unknown_names_and_versions_are_actionable() {
        let registry = registry();
        let error = registry.resolve("postgres@16").expect_err("unknown name");
        assert!(error.to_string().contains("available packages: demo"));

        let error = registry.resolve("demo@2").expect_err("unknown version");
        assert!(error.to_string().contains("available versions: 1.2, 1.10"));
    }

    #[test]
    fn hashes_are_normalized_and_platform_is_required() {
        let registry = registry();
        let resolved = registry.resolve("demo@1.2").expect("resolve");
        assert_eq!(resolved.platform.archive, ArchiveKind::Zip);
        assert_eq!(
            resolved.platform.sha256,
            "a".repeat(64),
            "hash must be lowercase"
        );
        assert_eq!(resolved.platform.binaries.len(), 3);
        let root = std::path::Path::new(r"C:\cache");
        let binary = resolved
            .binary_path(root, "server")
            .expect("binary role exists");
        assert!(binary.ends_with("server.exe"));
        assert_eq!(WINDOWS_X86_64, "windows-x86_64");
    }

    #[test]
    fn invalid_documents_are_rejected() {
        let mut registry = Registry::default();
        let error = registry
            .add_toml(
                &SAMPLE.replace(
                    "sha256 = \"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"",
                    "sha256 = \"not-a-hash\"",
                ),
                "bad-hash",
            )
            .expect_err("bad hash");
        assert!(error.to_string().contains("sha256"), "{error}");

        let error = registry
            .add_toml(
                &SAMPLE.replace(
                    "server = \"bin/server.exe\"\ninit",
                    "server = \"../server.exe\"\ninit",
                ),
                "traversal",
            )
            .expect_err("path traversal");
        assert!(error.to_string().contains("relative path"), "{error}");

        let error = registry
            .add_toml(
                &SAMPLE.replace("archive = \"zip\"", "archive = \"rar\""),
                "bad-archive",
            )
            .expect_err("bad archive");
        assert!(error.to_string().contains("archive type"), "{error}");
    }

    #[test]
    fn unsupported_native_entries_explain_the_fallback() {
        let text = r#"
schema = 1
name = "redis"
description = "Redis-compatible server"
license = "AGPL-3.0"
upstream = "https://redis.io/"
source = "https://redis.io/downloads/"
unsupported_native = "no official Windows build; use a container or Memurai"
"#;
        let mut registry = Registry::default();
        registry.add_toml(text, "redis").expect("parse");
        let error = registry.resolve("redis@7").expect_err("unsupported");
        let text = error.to_string();
        assert!(!text.is_empty());
        assert!(text.contains("not supported natively"), "{text}");
        assert!(text.contains("container"), "{text}");
    }
}
