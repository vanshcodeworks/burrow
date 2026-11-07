//! Redacted diagnostic bundles written by `burrow doctor --report`.
//!
//! A bundle is a local zip. It is never uploaded by Burrow. Every text entry is
//! passed through [`Redactor`], which removes known secrets, state tokens,
//! passwords in URLs and common `key=value` patterns, and user-specific paths.
//! Service logs are deliberately not copied: they can contain arbitrary output
//! from applications Burrow does not control.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use burrow_pkg::Cache;
use serde::Serialize;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::crash;
use crate::project::{Project, SecretsFile};
use crate::state::RuntimeState;

/// Inputs for one diagnostic bundle.
pub struct BundleOptions<'a> {
    /// Canonical project directory.
    pub project_dir: &'a Path,
    /// Parsed project, when `burrow.toml` was valid.
    pub project: Option<&'a Project>,
    /// Runtime state, live or read from disk.
    pub state: Option<&'a RuntimeState>,
    /// Doctor report as JSON. It may already contain a report path.
    pub report: serde_json::Value,
    /// RFC 3339 timestamp used in the report and README.
    pub generated_at: &'a str,
}

/// Chooses and creates the directory for a bundle, returning a unique path.
///
/// The default is `<project>/.burrow/reports`. `report_dir` overrides it and is
/// interpreted relative to the current directory.
pub fn bundle_path(project_dir: &Path, report_dir: Option<&Path>) -> Result<PathBuf> {
    let directory = match report_dir {
        Some(directory) => directory.to_path_buf(),
        None => project_dir.join(".burrow").join("reports"),
    };
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("could not create {}", directory.display()))?;
    let stamp = humantime::format_rfc3339_seconds(std::time::SystemTime::now())
        .to_string()
        .replace(':', "-");
    let base = format!("burrow-report-{stamp}-{}", std::process::id());
    for attempt in 0..100u32 {
        let name = if attempt == 0 {
            format!("{base}.zip")
        } else {
            format!("{base}-{attempt}.zip")
        };
        let path = directory.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    bail!(
        "could not choose a unique report name in {}",
        directory.display()
    )
}

/// Writes a redacted bundle to `destination`.
///
/// The archive is written to a temporary file first and renamed only after it
/// is complete, so an interrupted run never leaves a half-valid zip under the
/// final name.
pub fn write_bundle(options: &BundleOptions<'_>, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let temporary = destination.with_extension("zip.tmp");
    match write_archive(options, &temporary) {
        Ok(()) => {
            std::fs::rename(&temporary, destination)
                .with_context(|| format!("could not replace {}", destination.display()))?;
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn write_archive(options: &BundleOptions<'_>, path: &Path) -> Result<()> {
    let file = File::create(path).with_context(|| format!("could not write {}", path.display()))?;
    let mut zip = ZipWriter::new(file);
    let redactor = build_redactor(options);
    let crashes = crash::recent(options.project_dir, 10);

    let readme = readme(options, crashes.len(), &redactor);
    add_text(&mut zip, "README.txt", &readme)?;

    let report = serde_json::to_string_pretty(&options.report)
        .context("could not encode the doctor report")?;
    add_text(&mut zip, "report.json", &redactor.redact(&report))?;

    add_text(
        &mut zip,
        "environment.json",
        &environment_json(options, &redactor)?,
    )?;

    if let Some(project) = options.project {
        add_text(
            &mut zip,
            "config.redacted.json",
            &redacted_config(project, &redactor)?,
        )?;
    }

    if let Some(state) = options.state {
        let state = serde_json::to_string_pretty(&state.public_json())
            .context("could not encode the runtime state")?;
        add_text(&mut zip, "state.redacted.json", &redactor.redact(&state))?;
    }

    add_text(
        &mut zip,
        "daemon-log.txt",
        &daemon_log_tail(options.project_dir, &redactor),
    )?;

    for stored in &crashes {
        let name = stored
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("crash.json");
        let encoded =
            serde_json::to_string_pretty(&stored.note).context("could not encode a crash note")?;
        add_text(
            &mut zip,
            &format!("crashes/{name}"),
            &redactor.redact(&encoded),
        )?;
    }

    let file = zip
        .finish()
        .context("could not finish the diagnostic bundle")?;
    file.sync_all().ok();
    Ok(())
}

fn add_text(zip: &mut ZipWriter<File>, name: &str, contents: &str) -> Result<()> {
    zip.start_file(name, SimpleFileOptions::default())
        .with_context(|| format!("could not add {name} to the bundle"))?;
    zip.write_all(contents.as_bytes())
        .with_context(|| format!("could not write {name} in the bundle"))?;
    Ok(())
}

fn readme(options: &BundleOptions<'_>, crash_count: usize, redactor: &Redactor) -> String {
    let project = options
        .project
        .map(|project| project.config.project.name.as_str())
        .unwrap_or("<no valid burrow.toml>");
    let mut text = format!(
        "Burrow diagnostic bundle\n\
         ========================\n\n\
         Generated: {generated}\n\
         Burrow version: {version}\n\
         Project: {project}\n\n\
         This bundle was created locally by `burrow doctor --report`.\n\
         Nothing was uploaded anywhere. Review it before sharing it.\n\n\
         Contents\n\
         --------\n\
         README.txt             this file\n\
         report.json            the same report as `burrow doctor --json`\n\
         environment.json       OS, architecture, paths, and service-log sizes\n\
         config.redacted.json   burrow.toml with environment values and command\n\
                                arguments replaced\n\
         state.redacted.json    .burrow/state.json without the IPC token\n\
         daemon-log.txt         tail of .burrow/daemon.log\n\
         crashes/               up to 10 recent crash notes for this project\n\n\
         This bundle contains {crash_count} crash note(s).\n\n\
         Redaction\n\
         ---------\n\
         Known secrets, the daemon IPC token, URL passwords, Bearer tokens, and\n\
         common password/token/secret assignments are replaced with\n\
         `<redacted>`. The project directory, your home directory, and\n\
         %LOCALAPPDATA% are replaced with placeholders. Redaction is best-effort:\n\
         text Burrow did not write can still contain sensitive data.\n\n\
         Service logs are intentionally excluded because applications can print\n\
         arbitrary secrets. environment.json lists their file names and sizes so\n\
         a maintainer can ask for the relevant one.\n",
        generated = options.generated_at,
        version = env!("CARGO_PKG_VERSION"),
    );
    // Keep the bundle consistent even if a project name or a future field
    // happens to contain a known secret.
    text = redactor.redact(&text);
    text
}

fn environment_json(options: &BundleOptions<'_>, redactor: &Redactor) -> Result<String> {
    let burrow_dir = options
        .project
        .map(|project| redactor.redact(&project.burrow_dir.display().to_string()));
    let cache_dir = Cache::discover()
        .ok()
        .map(|cache| redactor.redact(&cache.root().display().to_string()));
    let logs = service_logs(options.project, redactor);
    let value = serde_json::json!({
        "burrow_version": env!("CARGO_PKG_VERSION"),
        "generated_at": options.generated_at,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "elevated": crate::win::is_elevated(),
        "project_dir": redactor.redact(&options.project_dir.display().to_string()),
        "burrow_dir": burrow_dir,
        "cache_dir": cache_dir,
        "state_file": options.state.is_some(),
        "service_logs": logs,
    });
    Ok(redactor.redact(&serde_json::to_string_pretty(&value)?))
}

#[derive(Serialize)]
struct LogFile {
    name: String,
    bytes: u64,
}

fn service_logs(project: Option<&Project>, redactor: &Redactor) -> Vec<LogFile> {
    let Some(project) = project else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(project.logs_dir()) else {
        return Vec::new();
    };
    let mut logs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("log") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        logs.push(LogFile {
            name: redactor.redact(name),
            bytes: metadata.len(),
        });
    }
    logs.sort_by(|left, right| left.name.cmp(&right.name));
    logs
}

fn redacted_config(project: &Project, redactor: &Redactor) -> Result<String> {
    let mut value = serde_json::to_value(&project.config)
        .context("could not encode the project configuration")?;
    if let Some(services) = value
        .get_mut("services")
        .and_then(serde_json::Value::as_object_mut)
    {
        for service in services.values_mut() {
            let Some(object) = service.as_object_mut() else {
                continue;
            };
            if let Some(env) = object
                .get_mut("env")
                .and_then(serde_json::Value::as_object_mut)
            {
                for value in env.values_mut() {
                    *value = serde_json::Value::String("<redacted>".to_owned());
                }
            }
            if let Some(run) = object.get_mut("run") {
                if let Some(command) = run.as_str() {
                    *run = serde_json::Value::String(redact_command(command));
                }
            }
            if let Some(health) = object
                .get_mut("health")
                .and_then(serde_json::Value::as_object_mut)
            {
                let redacted = health
                    .get("cmd")
                    .and_then(|value| value.as_str())
                    .map(redact_command);
                if let Some(redacted) = redacted {
                    health.insert("cmd".to_owned(), serde_json::Value::String(redacted));
                }
            }
        }
    }
    let pretty = serde_json::to_string_pretty(&value)
        .context("could not encode the redacted configuration")?;
    Ok(redactor.redact(&pretty))
}

fn redact_command(command: &str) -> String {
    match crate::commandline::split(command) {
        Ok((program, arguments)) if !arguments.is_empty() => {
            let mut redacted = program;
            for _ in arguments {
                redacted.push_str(" <redacted>");
            }
            redacted
        }
        Ok((program, _)) => program,
        Err(_) => "<redacted command>".to_owned(),
    }
}

fn daemon_log_tail(project_dir: &Path, redactor: &Redactor) -> String {
    let path = project_dir.join(".burrow").join("daemon.log");
    match read_tail(&path, 256 * 1024) {
        Some(text) if !text.trim().is_empty() => redactor.redact(&text),
        _ => "no daemon log was found\n".to_owned(),
    }
}

fn read_tail(path: &Path, max_bytes: u64) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let start = metadata.len().saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start == 0 {
        return Some(text);
    }
    // The first line is probably partial; drop it.
    Some(
        text.split_once('\n')
            .map(|(_, rest)| rest.to_owned())
            .unwrap_or_default(),
    )
}

fn build_redactor(options: &BundleOptions<'_>) -> Redactor {
    let mut redactor = Redactor::new(options.project_dir);
    if let Some(project) = options.project {
        for service in project.config.services.values() {
            for (name, value) in &service.env {
                if is_sensitive_name(name) {
                    redactor.add_secret(value);
                }
            }
        }
        let path = project.burrow_dir.join("secrets");
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Ok(document) = serde_json::from_str::<SecretsFile>(&contents) {
                for value in document.secrets.values() {
                    redactor.add_secret(value);
                }
            }
        }
    }
    if let Some(state) = options.state {
        redactor.add_secret(&state.token);
    }
    for (name, value) in std::env::vars_os() {
        if is_sensitive_name(&name.to_string_lossy()) {
            if let Some(value) = value.to_str() {
                redactor.add_secret(value);
            }
        }
    }
    redactor
}

/// Environment names whose values are treated as secrets everywhere.
fn is_sensitive_name(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "API_KEY",
        "APIKEY",
        "PRIVATE_KEY",
        "CREDENTIAL",
        "AUTH",
    ]
    .iter()
    .any(|needle| name.contains(needle))
}

/// Replaces known values and user-specific paths in text.
struct Redactor {
    /// Longest first, so a secret that contains another secret is fully removed.
    secrets: Vec<String>,
    /// `(needle, replacement)`, most specific first.
    paths: Vec<(String, &'static str)>,
}

impl Redactor {
    fn new(project_dir: &Path) -> Self {
        let mut paths = Vec::new();
        push_path_variants(&mut paths, project_dir, "<project>");
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            push_path_variants(&mut paths, Path::new(&local), "%LOCALAPPDATA%");
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            push_path_variants(&mut paths, Path::new(&home), "%USERPROFILE%");
        }
        Self {
            secrets: Vec::new(),
            paths,
        }
    }

    fn add_secret(&mut self, value: &str) {
        let value = value.trim();
        // Very short values would mangle ordinary text (for example `1` or
        // `true`) without protecting a meaningful credential.
        if value.len() < 6 {
            return;
        }
        self.push_secret(value.to_owned());
        if let Ok(encoded) = serde_json::to_string(value) {
            if encoded.len() > 2 {
                self.push_secret(encoded[1..encoded.len() - 1].to_owned());
            }
        }
    }

    fn push_secret(&mut self, value: String) {
        if !self.secrets.iter().any(|existing| existing == &value) {
            self.secrets.push(value);
            self.secrets
                .sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        }
    }

    fn redact(&self, text: &str) -> String {
        let mut value = text.to_owned();
        for (path, replacement) in &self.paths {
            if value.contains(path) {
                value = value.replace(path, replacement);
            }
        }
        for secret in &self.secrets {
            if secret.len() >= 8 {
                value = value.replace(secret, "<redacted>");
            } else {
                value = replace_delimited(&value, secret);
            }
        }
        let value = redact_url_passwords(&value);
        let value = redact_bearer_tokens(&value);
        redact_assignments(&value)
    }
}

fn push_path_variants(
    paths: &mut Vec<(String, &'static str)>,
    path: &Path,
    replacement: &'static str,
) {
    let display = path.display().to_string();
    if display.is_empty() {
        return;
    }
    let mut variants = vec![display.clone()];
    if let Some(rest) = display.strip_prefix(r"\\?\UNC\") {
        variants.push(format!(r"\\{rest}"));
    } else if let Some(rest) = display.strip_prefix(r"\\?\") {
        variants.push(rest.to_owned());
    }
    for variant in variants.clone() {
        if variant.contains('\\') {
            variants.push(variant.replace('\\', "/"));
        }
    }
    // JSON text doubles backslashes, so a path in `report.json` or a crash note
    // does not match the raw form. Add the escaped form too.
    let escaped = variants
        .iter()
        .filter_map(|variant| serde_json::to_string(variant).ok())
        .filter_map(|encoded| (encoded.len() > 2).then(|| encoded[1..encoded.len() - 1].to_owned()))
        .collect::<Vec<_>>();
    variants.extend(escaped);
    for variant in variants {
        if variant.len() >= 3 && !paths.iter().any(|(existing, _)| existing == &variant) {
            paths.push((variant, replacement));
        }
    }
}

fn replace_delimited(text: &str, needle: &str) -> String {
    if needle.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find(needle) {
        let index = cursor + relative;
        let after = index + needle.len();
        let before_ok = text[..index]
            .chars()
            .next_back()
            .is_none_or(|character| !is_word_character(character));
        let after_ok = text[after..]
            .chars()
            .next()
            .is_none_or(|character| !is_word_character(character));
        if before_ok && after_ok {
            out.push_str(&text[cursor..index]);
            out.push_str("<redacted>");
        } else {
            out.push_str(&text[cursor..after]);
        }
        cursor = after;
    }
    out.push_str(&text[cursor..]);
    out
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '-'
}

fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let hay = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    (from..=last).find(|&index| {
        haystack.is_char_boundary(index)
            && hay[index..index + needle.len()].eq_ignore_ascii_case(needle)
    })
}

fn redact_url_passwords(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(scheme) = rest.find("://") {
        let authority_start = scheme + 3;
        let authority_len = rest[authority_start..]
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(
                        character,
                        '/' | '?' | '#' | '"' | '\'' | '<' | '>' | ')' | ']' | '}' | ','
                    )
            })
            .unwrap_or(rest.len() - authority_start);
        let authority_end = authority_start + authority_len;
        let authority = &rest[authority_start..authority_end];
        if let Some(at) = authority.rfind('@') {
            if let Some(colon) = authority[..at].find(':') {
                let password_start = authority_start + colon + 1;
                let password_end = authority_start + at;
                out.push_str(&rest[..password_start]);
                out.push_str("<redacted>");
                rest = &rest[password_end..];
                continue;
            }
        }
        out.push_str(&rest[..authority_start]);
        rest = &rest[authority_start..];
    }
    out.push_str(rest);
    out
}

fn redact_bearer_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = find_ci(rest, "bearer ", 0) {
        out.push_str(&rest[..index]);
        out.push_str(&rest[index..index + 7]);
        let value_start = index + 7;
        let value_end = rest[value_start..]
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '"' | '\'' | ',' | ';' | '}' | ']' | ')')
            })
            .map(|offset| value_start + offset)
            .unwrap_or(rest.len());
        if value_end > value_start {
            out.push_str("<redacted>");
        }
        rest = &rest[value_end..];
    }
    out.push_str(rest);
    out
}

const ASSIGNMENT_KEYS: [&str; 13] = [
    "authorization",
    "client_secret",
    "private_key",
    "access_token",
    "refresh_token",
    "passphrase",
    "password",
    "passwd",
    "api_key",
    "api-key",
    "apikey",
    "secret",
    "token",
];

fn redact_assignments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    while cursor < text.len() {
        let mut earliest: Option<(usize, &str)> = None;
        for key in ASSIGNMENT_KEYS {
            if let Some(index) = find_ci(text, key, cursor) {
                if earliest.is_none_or(|(best, _)| index < best) {
                    earliest = Some((index, key));
                }
            }
        }
        let Some((index, key)) = earliest else {
            break;
        };
        let Some((value_start, value_end)) = assignment_value(text, index, key.len()) else {
            out.push_str(&text[cursor..index + 1]);
            cursor = index + 1;
            continue;
        };
        out.push_str(&text[cursor..value_start]);
        out.push_str("<redacted>");
        cursor = value_end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Finds the value span after `key` when it looks like `key=value` or
/// `"key": "value"`. Returns `(start, end)` with quotes excluded.
fn assignment_value(text: &str, key_start: usize, key_len: usize) -> Option<(usize, usize)> {
    if text[..key_start]
        .chars()
        .next_back()
        .is_some_and(|character| character.is_alphanumeric())
    {
        return None;
    }
    let bytes = text.as_bytes();
    let mut index = key_start + key_len;
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    if index < bytes.len() && matches!(bytes[index], b'"' | b'\'') {
        index += 1;
    }
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    if index >= bytes.len() || !matches!(bytes[index], b':' | b'=') {
        return None;
    }
    index += 1;
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    let value_start = index;
    if value_start >= bytes.len() {
        return None;
    }
    let quote = bytes[value_start];
    if quote == b'"' || quote == b'\'' {
        let mut end = value_start + 1;
        let mut escaped = false;
        while end < bytes.len() {
            let byte = bytes[end];
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote {
                break;
            }
            end += 1;
        }
        (end > value_start + 1).then_some((value_start + 1, end))
    } else {
        let mut end = value_start;
        while end < bytes.len() {
            let character = text[end..].chars().next().expect("valid character");
            if character.is_whitespace()
                || matches!(
                    character,
                    ',' | ';' | '}' | ']' | ')' | '&' | '\n' | '\r' | '"' | '\''
                )
            {
                break;
            }
            end += character.len_utf8();
        }
        (end > value_start).then_some((value_start, end))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Read;

    use super::{bundle_path, write_bundle, BundleOptions, Redactor};
    use crate::project::Project;
    use crate::state::{RuntimeState, ServiceSnapshot, ServiceStatus};

    const PROJECT: &str = r"C:\Users\example\AppData\Local\Temp\shop";

    fn redactor() -> Redactor {
        let mut redactor = Redactor::new(std::path::Path::new(PROJECT));
        redactor.add_secret("known-secret-0123456789");
        redactor
    }

    #[test]
    fn redacts_secrets_paths_and_common_patterns() {
        let text = format!(
            "dir {PROJECT} secret known-secret-0123456789\n\
             postgres://user:url-secret@127.0.0.1:5432/db\n\
             Authorization: Bearer bearer-secret\n\
             password=assignment-secret token: \"quoted-secret\"\n\
             harmless value stays"
        );
        let redacted = redactor().redact(&text);
        for secret in [
            "known-secret-0123456789",
            "url-secret",
            "bearer-secret",
            "assignment-secret",
            "quoted-secret",
            PROJECT,
        ] {
            assert!(!redacted.contains(secret), "leaked {secret}:\n{redacted}");
        }
        assert!(redacted.contains("<project>"));
        assert!(redacted.contains("<redacted>"));
        assert!(redacted.contains("harmless value stays"));
    }

    #[test]
    fn short_secrets_only_match_whole_words() {
        let mut redactor = Redactor::new(std::path::Path::new(PROJECT));
        redactor.add_secret("abcdef");
        let redacted = redactor.redact("xabcdefx abcdef");
        assert_eq!(redacted, "xabcdefx <redacted>");
    }

    #[test]
    fn redaction_keeps_json_valid() {
        let text = serde_json::json!({
            "project_dir": PROJECT,
            "message": "password=crash-secret",
            "detail": "token=report-secret",
        })
        .to_string();
        let redacted = redactor().redact(&text);
        let value: serde_json::Value = serde_json::from_str(&redacted).expect("valid JSON");
        assert_eq!(value["project_dir"], "<project>");
        assert_eq!(value["message"], "password=<redacted>");
        assert_eq!(value["detail"], "token=<redacted>");
    }

    #[test]
    fn ordinary_text_is_unchanged() {
        assert_eq!(
            redactor().redact("doctor found 2 problems in the project"),
            "doctor found 2 problems in the project"
        );
    }

    fn project(dir: &std::path::Path) -> Project {
        Project::load(Some(dir.to_path_buf())).expect("load project")
    }

    fn state(dir: &std::path::Path) -> RuntimeState {
        let mut services = std::collections::BTreeMap::new();
        services.insert(
            "db".to_owned(),
            ServiceSnapshot {
                status: ServiceStatus::Healthy,
                pid: Some(42),
                port: 5432,
                url: "postgres://user:url-secret@127.0.0.1:5432".to_owned(),
                route: None,
                route_url: None,
                restarts: 0,
                started_at: Some(1),
                exit_code: None,
                memory_bytes: 0,
                cpu_percent: 0.0,
                last_error: Some("token=state-error-secret".to_owned()),
                runtime: Some("postgres@16".to_owned()),
            },
        );
        RuntimeState {
            version: 1,
            project: "shop".to_owned(),
            project_dir: dir.to_path_buf(),
            pid: 7,
            token: "state-token-0123456789".to_owned(),
            control_port: 49152,
            proxy_port: None,
            started_at: 1,
            config_hash: "abc".to_owned(),
            ready: true,
            services,
        }
    }

    fn make_project(dir: &std::path::Path) -> Project {
        fs::write(
            dir.join("burrow.toml"),
            "[project]\nname = \"shop\"\n\n\
             [services.db]\nrun = 'node server.js --token cmd-secret'\n\n\
             [services.db.env]\nDATABASE_PASSWORD = \"env-secret-value\"\nLOG_LEVEL = \"debug\"\n",
        )
        .expect("write config");
        fs::create_dir_all(dir.join(".burrow").join("logs")).expect("logs dir");
        fs::write(
            dir.join(".burrow").join("secrets"),
            r#"{"version":0,"secrets":{"db":"known-secret-0123456789"}}"#,
        )
        .expect("write secrets");
        fs::write(
            dir.join(".burrow").join("daemon.log"),
            format!("started\npassword=log-secret\npath {}\n", dir.display()),
        )
        .expect("write daemon log");
        fs::write(
            dir.join(".burrow").join("logs").join("db.log"),
            "service output\n",
        )
        .expect("write service log");
        project(dir)
    }

    fn bundle(dir: &std::path::Path, report_dir: Option<&std::path::Path>) -> std::path::PathBuf {
        let project = make_project(dir);
        let state = state(dir);
        let report = serde_json::json!({
            "ok": true,
            "generated_at": "2026-10-26T12:00:00Z",
            "burrow_version": "0.1.0",
            "project": "shop",
            "project_dir": dir.display().to_string(),
            "checks": [{
                "name": "config",
                "status": "warn",
                "detail": "token=report-secret",
            }],
        });
        let destination = bundle_path(dir, report_dir).expect("bundle path");
        write_bundle(
            &BundleOptions {
                project_dir: dir,
                project: Some(&project),
                state: Some(&state),
                report,
                generated_at: "2026-10-26T12:00:00Z",
            },
            &destination,
        )
        .expect("write bundle");
        destination
    }

    fn entries(path: &std::path::Path) -> Vec<(String, String)> {
        let file = fs::File::open(path).expect("open zip");
        let mut archive = zip::ZipArchive::new(file).expect("read zip");
        let mut entries = Vec::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).expect("zip entry");
            let mut contents = String::new();
            entry.read_to_string(&mut contents).expect("read entry");
            let name = entry.name().expect("valid entry name").into_owned();
            entries.push((name, contents));
        }
        entries
    }

    #[test]
    fn bundle_contains_expected_entries_and_no_known_secrets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = bundle(dir.path(), None);
        assert!(path.is_file());
        assert!(!path.with_extension("zip.tmp").exists());
        let entries = entries(&path);
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        let project_path = dir.path().display().to_string();
        for expected in [
            "README.txt",
            "report.json",
            "environment.json",
            "config.redacted.json",
            "state.redacted.json",
            "daemon-log.txt",
        ] {
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
        for (name, contents) in &entries {
            for secret in [
                "known-secret-0123456789",
                "env-secret-value",
                "cmd-secret",
                "report-secret",
                "log-secret",
                "url-secret",
                "state-error-secret",
                "state-token-0123456789",
            ] {
                assert!(
                    !contents.contains(secret),
                    "{name} leaked {secret}:\n{contents}"
                );
            }
            assert!(
                !contents.contains(&project_path),
                "{name} leaked the project path:\n{contents}"
            );
        }
        let config = entries
            .iter()
            .find(|(name, _)| name == "config.redacted.json")
            .map(|(_, contents)| contents.as_str())
            .expect("config entry");
        assert!(config.contains("<redacted>"));
        assert!(config.contains("node <redacted>"));
        assert!(config.contains("LOG_LEVEL"));
    }

    #[test]
    fn bundle_without_a_project_still_has_core_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let destination = bundle_path(dir.path(), None).expect("bundle path");
        write_bundle(
            &BundleOptions {
                project_dir: dir.path(),
                project: None,
                state: None,
                report: serde_json::json!({"ok": false}),
                generated_at: "2026-10-26T12:00:00Z",
            },
            &destination,
        )
        .expect("write bundle");
        let entries = entries(&destination);
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"README.txt"));
        assert!(names.contains(&"report.json"));
        assert!(names.contains(&"environment.json"));
        assert!(names.contains(&"daemon-log.txt"));
        assert!(!names.contains(&"config.redacted.json"));
        assert!(!names.contains(&"state.redacted.json"));
    }

    #[test]
    fn bundle_honors_an_explicit_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let reports = tempfile::tempdir().expect("reports");
        let path = bundle(dir.path(), Some(reports.path()));
        assert_eq!(path.parent(), Some(reports.path()));
        assert!(path.is_file());
    }
}
