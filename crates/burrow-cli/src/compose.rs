//! Convert a Docker Compose file into a starter `burrow.toml`.
//!
//! This implements a deliberately small YAML subset: indentation-based maps and
//! sequences, quoted scalars, inline `[a, b]` / `{ k = v }` collections, and
//! literal/folded block scalars. That covers the common shape of
//! `docker-compose.yml` files without adding a YAML dependency to the binary.
//! Unsupported syntax produces a clear error instead of a silent misread.
//!
//! Docker-only concepts that cannot be expressed in Burrow (image builds,
//! fixed host ports, networks, named volumes) are not copied. They are called
//! out in comments at the end of the generated file so the user can review
//! them.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::init::{Scaffold, ServicePreview};

// ---------------------------------------------------------------------------
// Minimal YAML reader
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Yaml {
    Null,
    Scalar(String),
    Map(BTreeMap<String, Yaml>),
    Seq(Vec<Yaml>),
}

impl Yaml {
    fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(map) => map.get(key),
            _ => None,
        }
    }

    fn scalar(&self) -> Option<&str> {
        match self {
            Yaml::Scalar(value) => Some(value),
            _ => None,
        }
    }

    fn strings(&self) -> Vec<String> {
        match self {
            Yaml::Scalar(value) => vec![value.clone()],
            Yaml::Seq(items) => items
                .iter()
                .filter_map(|item| item.scalar().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct Line {
    number: usize,
    indent: usize,
    text: String,
}

fn strip_comment(input: &str) -> String {
    let mut quote: Option<char> = None;
    let mut previous_was_space = true;
    for (index, character) in input.char_indices() {
        match quote {
            Some(open) => {
                if character == open {
                    quote = None;
                }
            }
            None => match character {
                '\'' | '"' => quote = Some(character),
                '#' if previous_was_space => return input[..index].trim_end().to_owned(),
                _ => {}
            },
        }
        previous_was_space = character.is_whitespace();
    }
    input.trim_end().to_owned()
}

fn unquote(input: &str) -> String {
    let input = input.trim();
    let bytes = input.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0] as char;
        let last = *bytes.last().unwrap() as char;
        if (first == '"' || first == '\'') && first == last {
            let inner = &input[1..input.len() - 1];
            return if first == '"' {
                inner
                    .replace("\\n", "\n")
                    .replace("\\\"", "\"")
                    .replace("\\\\", "\\")
            } else {
                inner.replace("''", "'")
            };
        }
    }
    input.to_owned()
}

fn split_top_level(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (index, character) in input.char_indices() {
        match quote {
            Some(open) => {
                if character == open {
                    quote = None;
                }
            }
            None => match character {
                '\'' | '"' => quote = Some(character),
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(input[start..index].to_owned());
                    start = index + 1;
                }
                _ => {}
            },
        }
    }
    parts.push(input[start..].to_owned());
    parts
}

/// Split `key: value` at the first colon that is outside quotes and brackets.
fn split_key_value(input: &str) -> Option<(String, String)> {
    let mut quote: Option<char> = None;
    let mut depth = 0i32;
    for (index, character) in input.char_indices() {
        match quote {
            Some(open) => {
                if character == open {
                    quote = None;
                }
            }
            None => match character {
                '\'' | '"' => quote = Some(character),
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                ':' if depth == 0 => {
                    let rest = &input[index + 1..];
                    if rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace) {
                        return Some((unquote(&input[..index]), rest.trim().to_owned()));
                    }
                }
                _ => {}
            },
        }
    }
    None
}

fn is_sequence_item(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

fn parse_value(text: &str) -> Result<Yaml> {
    let text = text.trim();
    if text.starts_with('[') && text.ends_with(']') {
        let inner = &text[1..text.len() - 1];
        return Ok(Yaml::Seq(
            split_top_level(inner)
                .into_iter()
                .filter(|part| !part.trim().is_empty())
                .map(|part| Yaml::Scalar(unquote(&part)))
                .collect(),
        ));
    }
    if text.starts_with('{') && text.ends_with('}') {
        let inner = &text[1..text.len() - 1];
        let mut map = BTreeMap::new();
        for part in split_top_level(inner) {
            if let Some((key, value)) = split_key_value(&part) {
                map.insert(key, Yaml::Scalar(unquote(&value)));
            }
        }
        return Ok(Yaml::Map(map));
    }
    Ok(Yaml::Scalar(unquote(text)))
}

fn parse_block(lines: &[Line], position: &mut usize, indent: usize) -> Result<Yaml> {
    if *position >= lines.len() || lines[*position].indent < indent {
        return Ok(Yaml::Null);
    }
    if is_sequence_item(&lines[*position].text) {
        parse_sequence(lines, position, indent)
    } else {
        parse_mapping(lines, position, indent)
    }
}

fn parse_sequence(lines: &[Line], position: &mut usize, indent: usize) -> Result<Yaml> {
    let mut items = Vec::new();
    while *position < lines.len()
        && lines[*position].indent == indent
        && is_sequence_item(&lines[*position].text)
    {
        let line = lines[*position].clone();
        let rest = line.text.strip_prefix('-').unwrap_or("").trim_start();
        *position += 1;

        if rest.is_empty() {
            if *position < lines.len() && lines[*position].indent > indent {
                let child_indent = lines[*position].indent;
                items.push(parse_block(lines, position, child_indent)?);
            } else {
                items.push(Yaml::Null);
            }
            continue;
        }

        if let Some((key, value)) = split_key_value(rest) {
            let mut map = BTreeMap::new();
            map.insert(key, parse_value(&value)?);
            if *position < lines.len() && lines[*position].indent > indent {
                let child_indent = lines[*position].indent;
                if let Yaml::Map(extra) = parse_mapping(lines, position, child_indent)? {
                    map.extend(extra);
                }
            }
            items.push(Yaml::Map(map));
        } else {
            items.push(parse_value(rest)?);
        }
    }
    Ok(Yaml::Seq(items))
}

fn parse_mapping(lines: &[Line], position: &mut usize, indent: usize) -> Result<Yaml> {
    let mut map = BTreeMap::new();
    while *position < lines.len()
        && lines[*position].indent == indent
        && !is_sequence_item(&lines[*position].text)
    {
        let line = lines[*position].clone();
        let Some((key, value)) = split_key_value(&line.text) else {
            bail!(
                "line {}: expected `key: value`, got `{}`",
                line.number,
                line.text
            );
        };
        *position += 1;

        let parsed = if value.is_empty() {
            if *position < lines.len() && lines[*position].indent > indent {
                let child_indent = lines[*position].indent;
                parse_block(lines, position, child_indent)?
            } else {
                Yaml::Null
            }
        } else if value == "|" || value == ">" || value == "|-" || value == ">-" {
            let folded = value.starts_with('>');
            let mut pieces = Vec::new();
            while *position < lines.len() && lines[*position].indent > indent {
                pieces.push(lines[*position].text.clone());
                *position += 1;
            }
            Yaml::Scalar(pieces.join(if folded { " " } else { "\n" }))
        } else {
            parse_value(&value)?
        };
        map.insert(key, parsed);
    }
    Ok(Yaml::Map(map))
}

fn parse_yaml(text: &str) -> Result<Yaml> {
    let mut lines = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let leading: String = raw
            .chars()
            .take_while(|character| character.is_whitespace())
            .collect();
        if leading.contains('\t') {
            bail!(
                "line {}: tabs are not supported for YAML indentation",
                index + 1
            );
        }
        let indent = leading.chars().count();
        let content = strip_comment(raw.trim_start());
        if !content.is_empty() {
            lines.push(Line {
                number: index + 1,
                indent,
                text: content,
            });
        }
    }
    if lines.is_empty() {
        return Ok(Yaml::Null);
    }
    let mut position = 0;
    parse_block(&lines, &mut position, lines[0].indent)
}

// ---------------------------------------------------------------------------
// Compose → Burrow mapping
// ---------------------------------------------------------------------------

fn toml_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn toml_string(text: &str) -> String {
    if !text.contains('\'') && !text.contains('\n') && !text.contains('\\') {
        format!("'{text}'")
    } else {
        format!("\"{}\"", toml_escape(text))
    }
}

fn toml_key(text: &str) -> String {
    if !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        text.to_owned()
    } else {
        format!("\"{}\"", toml_escape(text))
    }
}

fn sanitize_name(name: &str) -> String {
    let mut result: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while result.contains("--") {
        result = result.replace("--", "-");
    }
    let result = result.trim_matches('-').to_owned();
    if result.is_empty() {
        "service".to_owned()
    } else {
        result
    }
}

fn builtin_use(image: &str) -> Option<String> {
    let (repository, tag) = match image.split_once(':') {
        Some((name, tag)) => (name, tag),
        None => (image, ""),
    };
    let package = repository.rsplit('/').next().unwrap_or(repository);
    let mut major = tag.split('.').next().unwrap_or("");
    if major.eq_ignore_ascii_case("latest") {
        major = "";
    }
    match package.to_ascii_lowercase().as_str() {
        "postgres" if major == "16" || major.is_empty() => Some("postgres@16".to_owned()),
        "nats" if major.is_empty() || major == "2" => Some("nats@2".to_owned()),
        "caddy" if major.is_empty() || major == "2" => Some("caddy@2".to_owned()),
        _ => None,
    }
}

fn command_string(value: &Yaml) -> Option<String> {
    match value {
        Yaml::Scalar(text) if !text.trim().is_empty() => Some(text.trim().to_owned()),
        Yaml::Seq(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|item| item.scalar())
                .map(shell_quote)
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" "))
            }
        }
        _ => None,
    }
}

fn shell_quote(text: &str) -> String {
    if !text.is_empty()
        && text
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_./:=@".contains(character))
    {
        text.to_owned()
    } else {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn environment(value: &Yaml) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    match value {
        Yaml::Map(map) => {
            for (key, item) in map {
                if let Some(text) = item.scalar() {
                    result.insert(key.clone(), text.to_owned());
                }
            }
        }
        Yaml::Seq(items) => {
            for item in items {
                if let Some(text) = item.scalar() {
                    if let Some((key, value)) = text.split_once('=') {
                        result.insert(key.trim().to_owned(), value.trim().to_owned());
                    }
                }
            }
        }
        _ => {}
    }
    result
}

fn needs(value: &Yaml) -> Vec<String> {
    match value {
        Yaml::Seq(items) => items
            .iter()
            .filter_map(|item| item.scalar().map(sanitize_name))
            .collect(),
        Yaml::Map(map) => map.keys().map(|key| sanitize_name(key)).collect(),
        _ => Vec::new(),
    }
}

fn seed_from_volumes(value: &Yaml) -> Option<String> {
    for entry in value.strings() {
        if entry.contains("/docker-entrypoint-initdb.d/") {
            let source = entry.split(':').next().unwrap_or("").trim();
            if source.to_ascii_lowercase().ends_with(".sql") {
                return Some(source.replace('\\', "/"));
            }
        }
    }
    None
}

fn health_inline(healthcheck: &Yaml) -> Option<String> {
    let command = healthcheck
        .get("test")
        .and_then(command_string)
        .unwrap_or_default();
    let mut parts = Vec::new();
    if command.contains("curl") || command.contains("wget") || command.contains("http") {
        let path = ["/healthz", "/health", "/ping", "/"]
            .into_iter()
            .find(|candidate| command.contains(candidate))
            .unwrap_or("/");
        parts.push(format!("http = \"{path}\""));
    } else {
        parts.push("tcp = true".to_owned());
    }
    if let Some(interval) = healthcheck.get("interval").and_then(Yaml::scalar) {
        parts.push(format!("interval = \"{}\"", toml_escape(interval)));
    }
    if let Some(timeout) = healthcheck.get("timeout").and_then(Yaml::scalar) {
        parts.push(format!("timeout = \"{}\"", toml_escape(timeout)));
    }
    Some(format!("{{ {} }}", parts.join(", ")))
}

fn restart_value(text: &str) -> Option<&'static str> {
    match text.to_ascii_lowercase().as_str() {
        "always" | "unless-stopped" => Some("always"),
        "on-failure" => Some("on-failure"),
        "no" => Some("never"),
        _ => None,
    }
}

fn memory_value(text: &str) -> Option<String> {
    let split = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    let amount: u64 = text[..split].parse().ok()?;
    let megabytes = match text[split..].trim().to_ascii_lowercase().as_str() {
        "m" | "mb" => amount,
        "g" | "gb" => amount.checked_mul(1024)?,
        "k" | "kb" => amount / 1024,
        _ => return None,
    };
    if megabytes == 0 {
        None
    } else {
        Some(format!("{megabytes}MB"))
    }
}

fn container_port(ports: &[String]) -> Option<u16> {
    for entry in ports {
        let pieces: Vec<&str> = entry.split(':').collect();
        if pieces.len() >= 2 {
            let target = pieces[1].split('/').next().unwrap_or("");
            if let Ok(port) = target.trim().parse::<u16>() {
                return Some(port);
            }
        }
    }
    None
}

/// Convert a Compose file into a `burrow.toml` scaffold.
pub(crate) fn from_compose(path: &Path, project: &str) -> Result<Scaffold> {
    let text =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    let root = parse_yaml(&text).with_context(|| format!("could not parse {}", path.display()))?;
    let services = match root.get("services") {
        Some(Yaml::Map(map)) => map,
        Some(_) => bail!("`services` in {} must be a mapping", path.display()),
        None => bail!("{} has no `services:` mapping", path.display()),
    };
    if services.is_empty() {
        bail!("{} defines no services", path.display());
    }

    let mut notes: Vec<String> = Vec::new();
    let mut preview: Vec<ServicePreview> = Vec::new();
    let mut config = String::new();
    config.push_str("# Generated by `burrow init --from docker-compose.yml`.\n");
    config.push_str("# Docker-only fields are not carried over; see the notes at the end.\n");
    config.push_str(&format!(
        "# Source: {}\n\n",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    config.push_str("[project]\n");
    config.push_str(&format!("name = \"{}\"\n\n", toml_escape(project)));

    for (raw_name, value) in services {
        let fields = match value {
            Yaml::Map(map) => map,
            _ => {
                notes.push(format!("{raw_name}: skipped because it is not a mapping"));
                continue;
            }
        };
        let name = sanitize_name(raw_name);
        if &name != raw_name {
            notes.push(format!("{raw_name}: service name became `{name}`"));
        }

        let image = fields
            .get("image")
            .and_then(Yaml::scalar)
            .map(str::to_owned);
        let command = fields.get("command").and_then(command_string);
        let ports = fields.get("ports").map(Yaml::strings).unwrap_or_default();
        let env = fields
            .get("environment")
            .map(environment)
            .unwrap_or_default();
        let dependencies = fields.get("depends_on").map(needs).unwrap_or_default();
        let seed = fields.get("volumes").and_then(seed_from_volumes);
        let health = fields.get("healthcheck").and_then(health_inline);
        let restart = fields
            .get("restart")
            .and_then(Yaml::scalar)
            .and_then(restart_value);
        let memory = fields
            .get("mem_limit")
            .and_then(Yaml::scalar)
            .and_then(memory_value);

        config.push_str(&format!("[services.{name}]\n"));
        let display;

        if let Some(reference) = &image {
            if let Some(spec) = builtin_use(reference) {
                config.push_str(&format!("use = \"{spec}\"\n"));
                display = format!("use {spec}");
                notes.push(format!(
                    "{name}: image `{reference}` mapped to built-in package `{spec}`"
                ));
                if command.is_some() {
                    notes.push(format!(
                        "{name}: compose `command` ignored for a managed package"
                    ));
                }
            } else {
                config.push_str(&format!("use = \"container:{}\"\n", toml_escape(reference)));
                display = format!("container {reference}");
                if let Some(port) = container_port(&ports) {
                    config.push_str(&format!("container_port = {port}\n"));
                }
            }
            if fields.contains_key("build") {
                notes.push(format!(
                    "{name}: compose `build` ignored; Burrow does not build images"
                ));
            }
        } else if let Some(command) = &command {
            config.push_str(&format!("run = {}\n", toml_string(command)));
            display = command.clone();
        } else {
            config.push_str(&format!("run = \"echo TODO: set a command for {name}\"\n"));
            display = "TODO: set a command".to_owned();
            notes.push(format!("{name}: no image or command; add a `run` command"));
        }

        config.push('\n');
        if !ports.is_empty() {
            config.push_str(&format!(
                "# compose ports: {}  -> Burrow allocates a stable port automatically\n",
                ports.join(", ")
            ));
            config.push_str(&format!(
                "# route = \"{name}.localhost\"  # uncomment for HTTP services\n"
            ));
        }
        if !env.is_empty() {
            let entries: Vec<String> = env
                .iter()
                .map(|(key, value)| format!("{} = {}", toml_key(key), toml_string(value)))
                .collect();
            config.push_str(&format!("env = {{ {} }}\n", entries.join(", ")));
            if env.values().any(|value| value.contains("${")) {
                notes.push(format!(
                    "{name}: `${{...}}` values are not expanded by Burrow; use {{{{service.field}}}} templates"
                ));
            }
        }
        if !dependencies.is_empty() {
            let values: Vec<String> = dependencies
                .iter()
                .map(|dependency| format!("\"{}\"", toml_escape(dependency)))
                .collect();
            config.push_str(&format!("needs = [{}]\n", values.join(", ")));
        }
        if let Some(path) = seed {
            config.push_str(&format!("seed = \"{}\"\n", toml_escape(&path)));
            notes.push(format!(
                "{name}: seed assumes `{path}` exists in the Burrow project directory"
            ));
        }
        if let Some(health) = health {
            config.push_str(&format!("health = {health}\n"));
        }
        if let Some(policy) = restart {
            config.push_str(&format!("restart = \"{policy}\"\n"));
        }
        if let Some(memory) = memory {
            config.push_str(&format!("memory = \"{memory}\"\n"));
        }
        if fields.contains_key("networks") {
            notes.push(format!(
                "{name}: compose networks ignored; Burrow uses loopback"
            ));
        }
        config.push('\n');

        preview.push(ServicePreview {
            name,
            command: display,
            route: None,
        });
    }

    if !notes.is_empty() {
        config.push_str("# Conversion notes:\n");
        for note in &notes {
            config.push_str(&format!("# - {note}\n"));
        }
    }

    Ok(Scaffold {
        template: "compose",
        summary: format!(
            "converted {} service(s) from {} ({} note(s))",
            preview.len(),
            path.file_name().unwrap_or_default().to_string_lossy(),
            notes.len()
        ),
        config,
        services: preview,
        extra_files: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_block_maps_and_sequences() {
        let yaml = r#"
services:
  db:
    image: postgres:16
    ports:
      - "5432:5432"
    environment:
      - POSTGRES_USER=postgres
"#;
        let root = parse_yaml(yaml).expect("parse");
        let services = match root.get("services").expect("services") {
            Yaml::Map(map) => map,
            _ => panic!("services should be a map"),
        };
        let db = services.get("db").expect("db");
        assert_eq!(db.get("image").and_then(Yaml::scalar), Some("postgres:16"));
        assert_eq!(db.get("ports").unwrap().strings(), vec!["5432:5432"]);
        assert_eq!(
            environment(db.get("environment").expect("env")).get("POSTGRES_USER"),
            Some(&"postgres".to_owned())
        );
    }

    #[test]
    fn maps_builtin_images_into_managed_packages() {
        assert_eq!(builtin_use("postgres:16"), Some("postgres@16".to_owned()));
        assert_eq!(builtin_use("postgres:17"), None);
        assert_eq!(builtin_use("nats:2.10"), Some("nats@2".to_owned()));
        assert_eq!(builtin_use("caddy:latest"), Some("caddy@2".to_owned()));
    }

    #[test]
    fn converts_a_realistic_compose_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("docker-compose.yml");
        fs::write(
            &file,
            r#"services:
  db:
    image: postgres:16
    volumes:
      - ./db/init.sql:/docker-entrypoint-initdb.d/init.sql
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 2s
  api:
    build: .
    command: node server.js
    environment:
      DATABASE_URL: postgres://postgres@db:5432/dev
    depends_on:
      - db
    ports:
      - "3000:3000"
    mem_limit: 512m
    restart: unless-stopped
"#,
        )
        .expect("write compose file");

        let scaffold = from_compose(&file, "demo").expect("convert");
        assert_eq!(scaffold.services.len(), 2);
        assert!(scaffold.config.contains("name = \"demo\""));
        assert!(scaffold.config.contains("use = \"postgres@16\""));
        assert!(scaffold.config.contains("seed = \"./db/init.sql\""));
        assert!(scaffold
            .config
            .contains("health = { tcp = true, interval = \"2s\" }"));
        assert!(scaffold.config.contains("run = 'node server.js'"));
        assert!(scaffold.config.contains("needs = [\"db\"]"));
        assert!(scaffold.config.contains("memory = \"512MB\""));
        assert!(scaffold.config.contains("restart = \"always\""));
        assert!(scaffold.config.contains("compose ports"));
        burrow_config::parse(&scaffold.config).expect("generated config validates");
    }

    #[test]
    fn rejects_a_file_without_services() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("compose.yml");
        fs::write(&file, "version: '3'\n").expect("write");
        assert!(from_compose(&file, "demo").is_err());
    }
}
