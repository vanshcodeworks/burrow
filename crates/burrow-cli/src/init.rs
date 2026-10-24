//! `burrow init`: detect the project stack and write a starter `burrow.toml`.
//!
//! Detection is deliberately conservative. A framework is only wired to a
//! fixed port when Burrow knows how to pass that port on the command line
//! (Vite and Next). For an unrecognized server the generated service receives
//! `PORT` but no route, with a comment explaining what to add after checking
//! that the server honors it.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::commands::Options;
use crate::project;

/// Directories that never contain application entry points.
const SKIPPED_DIRECTORIES: [&str; 9] = [
    ".git",
    ".burrow",
    ".venv",
    "venv",
    "node_modules",
    "__pycache__",
    "target",
    "dist",
    "build",
];

struct Scaffold {
    template: &'static str,
    summary: String,
    config: String,
    services: Vec<ServicePreview>,
    extra_files: Vec<(PathBuf, String)>,
}

#[derive(Debug, Serialize)]
struct ServicePreview {
    name: String,
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    route: Option<String>,
}

/// Implements `burrow init`.
pub fn run(options: &Options, template: Option<String>) -> Result<()> {
    let directory = project::resolve_directory(options.project.clone())?;

    let config_path = directory.join(burrow_config::CONFIG_FILE);
    if config_path.exists() {
        bail!(
            "{} already exists; refusing to overwrite it\n\
             help: edit that file, or delete it first if you want a fresh scaffold",
            config_path.display()
        );
    }

    let name = project_name(&directory);
    let scaffold = match template.as_deref() {
        Some("node") => node_scaffold(&directory, &name)?.with_context(|| {
            "the `node` template needs a package.json with a dev or start script"
        })?,
        Some("python") => python_scaffold(&directory, &name)?.with_context(|| {
            "the `python` template needs a Python entry point (main.py or app.py)"
        })?,
        Some("postgres") => postgres_scaffold(&name),
        Some("cargo") => cargo_scaffold(&directory, &name)?
            .with_context(|| "the `cargo` template needs Cargo.toml and src/main.rs")?,
        Some("go") => {
            go_scaffold(&directory, &name)?.with_context(|| "the `go` template needs go.mod")?
        }
        Some(other) => bail!(
            "unknown template `{other}`\n\
             help: choose one of node, python, postgres, cargo, go, or omit --template to detect"
        ),
        None => detect(&directory, &name)?,
    };

    // Fail before writing anything if the generated file is not valid.
    burrow_config::parse(&scaffold.config)
        .map_err(|error| anyhow::anyhow!("internal error: generated config is invalid: {error}"))?;

    let mut created = Vec::new();
    for (path, contents) in &scaffold.extra_files {
        let absolute = directory.join(path);
        if absolute.exists() {
            continue;
        }
        if let Some(parent) = absolute.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        create_new(&absolute, contents)?;
        created.push(relative_display(path));
    }
    create_new(&config_path, &scaffold.config)?;
    created.push(burrow_config::CONFIG_FILE.to_owned());

    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "template": scaffold.template,
                "summary": scaffold.summary,
                "project": name,
                "directory": directory,
                "config": config_path,
                "services": scaffold.services,
                "created": created,
            }))?
        );
    } else {
        println!(
            "Created {} for `{name}` ({}).",
            config_path.display(),
            scaffold.summary
        );
        for service in &scaffold.services {
            match &service.route {
                Some(route) => println!(
                    "  {:<8} {:<28} route {route}",
                    service.name, service.command
                ),
                None => println!("  {:<8} {}", service.name, service.command),
            }
        }
        println!("\nNext:");
        println!("  burrow doctor    # check commands, ports, and the cache");
        println!("  burrow up        # review the trust prompt, then start");
        println!("  burrow top       # live dashboard");
    }

    Ok(())
}

fn create_new(path: &Path, contents: &str) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("could not create {}", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

fn relative_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Derives a valid project name from the directory name.
fn project_name(directory: &Path) -> String {
    let raw = directory
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| "project".to_owned());
    let mut name = String::new();
    for character in raw.chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            name.push(character);
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    let name = name.trim_matches('-');
    let name = if name.is_empty() { "project" } else { name };
    name.chars()
        .take(63)
        .collect::<String>()
        .trim_end_matches('-')
        .to_owned()
}

fn detect(directory: &Path, name: &str) -> Result<Scaffold> {
    if let Some(scaffold) = node_scaffold(directory, name)? {
        return Ok(scaffold);
    }
    if let Some(scaffold) = python_scaffold(directory, name)? {
        return Ok(scaffold);
    }
    if let Some(scaffold) = cargo_scaffold(directory, name)? {
        return Ok(scaffold);
    }
    if let Some(scaffold) = go_scaffold(directory, name)? {
        return Ok(scaffold);
    }
    bail!(
        "could not detect a stack in {}\n\
         help: pass --template node|python|postgres|cargo|go",
        directory.display()
    )
}

// ---------------------------------------------------------------------------
// Node.js

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeFramework {
    Vite,
    Next,
    Express,
    Unknown,
}

impl NodeFramework {
    fn label(self) -> &'static str {
        match self {
            Self::Vite => "Vite",
            Self::Next => "Next.js",
            Self::Express => "Express",
            Self::Unknown => "unknown framework",
        }
    }
}

fn node_scaffold(directory: &Path, name: &str) -> Result<Option<Scaffold>> {
    let package_path = directory.join("package.json");
    if !package_path.is_file() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&package_path)
        .with_context(|| format!("could not read {}", package_path.display()))?;
    let package: serde_json::Value = serde_json::from_str(&contents)
        .with_context(|| format!("{} is not valid JSON", package_path.display()))?;

    let scripts = package.get("scripts").and_then(|value| value.as_object());
    let (script_name, script_body) = match scripts {
        Some(scripts) => match scripts.get("dev").or_else(|| scripts.get("start")) {
            Some(script) => (
                if scripts.contains_key("dev") {
                    "dev"
                } else {
                    "start"
                },
                script.as_str().unwrap_or_default(),
            ),
            None => return Ok(None),
        },
        None => return Ok(None),
    };

    let mut dependencies = BTreeMap::new();
    for key in ["dependencies", "devDependencies"] {
        if let Some(values) = package.get(key).and_then(|value| value.as_object()) {
            for (name, version) in values {
                dependencies.insert(name.clone(), version.clone());
            }
        }
    }
    let has = |name: &str| dependencies.contains_key(name);
    let framework = if has("next") || script_body.contains("next") {
        NodeFramework::Next
    } else if has("vite") || script_body.contains("vite") {
        NodeFramework::Vite
    } else if has("express") {
        NodeFramework::Express
    } else {
        NodeFramework::Unknown
    };

    let manager = package_manager(directory, &package);
    let (command, route, health, note) = match framework {
        NodeFramework::Vite => (
            run_command(
                manager,
                script_name,
                &["--port", "{{web.port}}", "--host", "127.0.0.1"],
            ),
            Some("web.localhost"),
            true,
            None,
        ),
        NodeFramework::Next => (
            run_command(manager, script_name, &["-p", "{{web.port}}"]),
            Some("web.localhost"),
            true,
            None,
        ),
        NodeFramework::Express => (
            run_command(manager, script_name, &[]),
            Some("web.localhost"),
            true,
            None,
        ),
        NodeFramework::Unknown => (
            run_command(manager, script_name, &[]),
            None,
            false,
            Some(
                "The dev server was not recognized. It receives PORT above; if it\n\
                 listens on that port, add `route = \"web.localhost\"` and\n\
                 `health = { tcp = true }`. Otherwise pass the port with your own\n\
                 command flags.",
            ),
        ),
    };

    let config = node_config(name, &command, route, health, note);
    Ok(Some(Scaffold {
        template: "node",
        summary: format!("Node.js · {} · {manager}", framework.label()),
        config,
        services: vec![ServicePreview {
            name: "web".to_owned(),
            command,
            route: route.map(str::to_owned),
        }],
        extra_files: Vec::new(),
    }))
}

fn package_manager(directory: &Path, package: &serde_json::Value) -> &'static str {
    if let Some(manager) = package
        .get("packageManager")
        .and_then(|value| value.as_str())
    {
        if manager.starts_with("pnpm") {
            return "pnpm";
        }
        if manager.starts_with("yarn") {
            return "yarn";
        }
        if manager.starts_with("bun") {
            return "bun";
        }
        if manager.starts_with("npm") {
            return "npm";
        }
    }
    if directory.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if directory.join("yarn.lock").is_file() {
        "yarn"
    } else if directory.join("bun.lock").is_file() || directory.join("bun.lockb").is_file() {
        "bun"
    } else {
        "npm"
    }
}

fn run_command(manager: &str, script: &str, arguments: &[&str]) -> String {
    let mut command = format!("{manager} run {script}");
    if arguments.is_empty() {
        return command;
    }
    // Yarn 1 passes trailing flags through without `--`; npm, pnpm, and bun
    // require the separator to avoid parsing the flags themselves.
    if manager != "yarn" {
        command.push_str(" --");
    }
    for argument in arguments {
        command.push(' ');
        command.push_str(argument);
    }
    command
}

fn node_config(
    name: &str,
    command: &str,
    route: Option<&str>,
    health: bool,
    note: Option<&str>,
) -> String {
    let mut config = String::new();
    config.push_str(
        "# Generated by `burrow init`. Review the command before approving it.\n\
         # Edit freely: this file is yours.\n\n",
    );
    config.push_str("[project]\nname = \"@@NAME@@\"\n\n[services.web]\n");
    config.push_str("run = '@@RUN@@'\n");
    config.push_str("env = { PORT = \"{{web.port}}\" }\n");
    if let Some(route) = route {
        config.push_str(&format!("route = \"{route}\"\n"));
    }
    if health {
        config.push_str(
            "health = { tcp = true, timeout = \"120s\", interval = \"500ms\" }\n\
             restart = \"on-failure\"\n",
        );
    }
    if let Some(note) = note {
        config.push_str("#\n");
        for line in note.lines() {
            config.push_str("# ");
            config.push_str(line);
            config.push('\n');
        }
    }
    config.replace("@@NAME@@", name).replace("@@RUN@@", command)
}

// ---------------------------------------------------------------------------
// Python

#[derive(Clone, Copy, PartialEq, Eq)]
enum PythonFramework {
    FastApi,
    Flask,
    Plain,
}

impl PythonFramework {
    fn label(self) -> &'static str {
        match self {
            Self::FastApi => "FastAPI",
            Self::Flask => "Flask",
            Self::Plain => "script",
        }
    }
}

struct PythonEntry {
    framework: PythonFramework,
    module: Option<String>,
    variable: String,
}

fn python_scaffold(directory: &Path, name: &str) -> Result<Option<Scaffold>> {
    let mut files = Vec::new();
    collect_python_files(directory, 0, &mut files);
    let has_python = !files.is_empty()
        || directory.join("pyproject.toml").is_file()
        || directory.join("requirements.txt").is_file();
    if !has_python {
        return Ok(None);
    }

    let entry = find_python_entry(&files, directory);
    let framework = entry
        .as_ref()
        .map(|entry| entry.framework)
        .unwrap_or(PythonFramework::Plain);
    let (command, route, health, note) = match entry {
        Some(PythonEntry {
            framework: PythonFramework::FastApi,
            module: Some(module),
            variable,
        }) => (
            format!(
                "python -m uvicorn {module}:{variable} --host 127.0.0.1 --port {{{{api.port}}}}"
            ),
            Some("api.localhost"),
            true,
            None,
        ),
        Some(PythonEntry {
            framework: PythonFramework::Flask,
            module: Some(module),
            ..
        }) => (
            format!("python -m flask --app {module} run --host 127.0.0.1 --port {{{{api.port}}}}"),
            Some("api.localhost"),
            true,
            None,
        ),
        Some(PythonEntry {
            framework: PythonFramework::FastApi,
            module: None,
            ..
        }) => (
            "python -m uvicorn main:app --host 127.0.0.1 --port {{api.port}}".to_owned(),
            Some("api.localhost"),
            true,
            Some("Check the module and variable in this command; uvicorn expects `module:app`."),
        ),
        Some(PythonEntry {
            framework: PythonFramework::Flask,
            module: None,
            ..
        }) => (
            "python -m flask --app main run --host 127.0.0.1 --port {{api.port}}".to_owned(),
            Some("api.localhost"),
            true,
            Some("Check the application module in this command."),
        ),
        _ => {
            if directory.join("main.py").is_file() {
                (
                    "python main.py".to_owned(),
                    None,
                    false,
                    Some(
                        "The script receives PORT above. If it listens on that port, add\n\
                         `route = \"api.localhost\"` and `health = { tcp = true }`.",
                    ),
                )
            } else {
                (
                    "python main.py".to_owned(),
                    None,
                    false,
                    Some(
                        "No entry point was recognized. Change the command to your module,\n\
                         for example `python -m myapp`.",
                    ),
                )
            }
        }
    };
    let config = python_config(name, &command, route, health, note);
    Ok(Some(Scaffold {
        template: "python",
        summary: format!("Python · {}", framework.label()),
        config,
        services: vec![ServicePreview {
            name: "api".to_owned(),
            command,
            route: route.map(str::to_owned),
        }],
        extra_files: Vec::new(),
    }))
}

fn python_config(
    name: &str,
    command: &str,
    route: Option<&str>,
    health: bool,
    note: Option<&str>,
) -> String {
    let mut config = String::new();
    config.push_str(
        "# Generated by `burrow init`. Review the command before approving it.\n\
         # Activate the project's virtual environment before `burrow up` if you use one.\n\n",
    );
    config.push_str("[project]\nname = \"@@NAME@@\"\n\n[services.api]\n");
    config.push_str("run = '@@RUN@@'\n");
    config.push_str("env = { PORT = \"{{api.port}}\" }\n");
    if let Some(route) = route {
        config.push_str(&format!("route = \"{route}\"\n"));
    }
    if health {
        config.push_str(
            "health = { tcp = true, timeout = \"120s\", interval = \"500ms\" }\n\
             restart = \"on-failure\"\n",
        );
    }
    if let Some(note) = note {
        config.push_str("#\n");
        for line in note.lines() {
            config.push_str("# ");
            config.push_str(line);
            config.push('\n');
        }
    }
    config.replace("@@NAME@@", name).replace("@@RUN@@", command)
}

fn collect_python_files(directory: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    if depth > 2 || files.len() > 200 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !SKIPPED_DIRECTORIES.contains(&name.as_ref()) {
                collect_python_files(&path, depth + 1, files);
            }
        } else if path.extension().is_some_and(|extension| extension == "py") {
            files.push(path);
        }
    }
    files.sort();
}

fn find_python_entry(files: &[PathBuf], root: &Path) -> Option<PythonEntry> {
    // Root `main.py` and `app.py` are the conventional entry points; prefer
    // them over nested modules so the generated command stays simple.
    let mut ordered = files.to_vec();
    ordered.sort_by_key(|path| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let depth = path.components().count();
        (depth, name != "main.py" && name != "app.py")
    });

    for path in ordered {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let framework = if contents.contains("FastAPI(") {
            PythonFramework::FastApi
        } else if contents.contains("Flask(") {
            PythonFramework::Flask
        } else {
            continue;
        };
        let variable = instance_name(&contents, framework);
        let module = module_name(&path, root);
        return Some(PythonEntry {
            framework,
            module,
            variable,
        });
    }
    None
}

fn instance_name(contents: &str, framework: PythonFramework) -> String {
    let needle = match framework {
        PythonFramework::FastApi => "FastAPI(",
        PythonFramework::Flask => "Flask(",
        PythonFramework::Plain => return "app".to_owned(),
    };
    for line in contents.lines() {
        let Some(index) = line.find(needle) else {
            continue;
        };
        if let Some(equal) = line[..index].rfind('=') {
            let left = line[..equal].trim();
            if let Some(name) = left.split_whitespace().next() {
                let name = name.trim_end_matches(':');
                if name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
                    && !name.is_empty()
                {
                    return name.to_owned();
                }
            }
        }
    }
    "app".to_owned()
}

/// Converts a Python file below `root` into a dotted import path.
///
/// `uvicorn` and `flask` run with the project root as their working directory,
/// so only path components below that root belong in the module name. A root
/// `main.py` is therefore `main`, not `project-name.main`.
fn module_name(path: &Path, root: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let mut parts = Vec::new();
    for component in relative.components() {
        let std::path::Component::Normal(part) = component else {
            continue;
        };
        parts.push(part.to_string_lossy().into_owned());
    }
    let last = parts.pop()?;
    let stem = last.strip_suffix(".py")?;
    if stem.is_empty() {
        return None;
    }
    if !parts.iter().all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    }) {
        return None;
    }
    if stem != "__init__" {
        parts.push(stem.to_owned());
    } else if parts.is_empty() {
        return None;
    }
    Some(parts.join("."))
}

// ---------------------------------------------------------------------------
// Cargo and Go

fn cargo_scaffold(directory: &Path, name: &str) -> Result<Option<Scaffold>> {
    if !directory.join("Cargo.toml").is_file() || !directory.join("src/main.rs").is_file() {
        return Ok(None);
    }
    let command = "cargo run".to_owned();
    let config = generic_config(
        name,
        &command,
        Some(
            "The program receives PORT above. If it listens on that port, add\n\
             `route = \"app.localhost\"` and `health = { tcp = true }`.",
        ),
    );
    Ok(Some(Scaffold {
        template: "cargo",
        summary: "Rust · Cargo binary".to_owned(),
        config,
        services: vec![ServicePreview {
            name: "app".to_owned(),
            command,
            route: None,
        }],
        extra_files: Vec::new(),
    }))
}

fn go_scaffold(directory: &Path, name: &str) -> Result<Option<Scaffold>> {
    if !directory.join("go.mod").is_file() || !directory.join("main.go").is_file() {
        return Ok(None);
    }
    let command = "go run .".to_owned();
    let config = generic_config(
        name,
        &command,
        Some(
            "The program receives PORT above. If it listens on that port, add\n\
             `route = \"app.localhost\"` and `health = { tcp = true }`.",
        ),
    );
    Ok(Some(Scaffold {
        template: "go",
        summary: "Go · main package".to_owned(),
        config,
        services: vec![ServicePreview {
            name: "app".to_owned(),
            command,
            route: None,
        }],
        extra_files: Vec::new(),
    }))
}

fn generic_config(name: &str, command: &str, note: Option<&str>) -> String {
    let mut config = String::new();
    config.push_str("# Generated by `burrow init`. Review the command before approving it.\n\n");
    config.push_str("[project]\nname = \"@@NAME@@\"\n\n[services.app]\n");
    config.push_str("run = '@@RUN@@'\n");
    config.push_str("env = { PORT = \"{{app.port}}\" }\n");
    if let Some(note) = note {
        config.push_str("#\n");
        for line in note.lines() {
            config.push_str("# ");
            config.push_str(line);
            config.push('\n');
        }
    }
    config.replace("@@NAME@@", name).replace("@@RUN@@", command)
}

// ---------------------------------------------------------------------------
// PostgreSQL

fn postgres_scaffold(name: &str) -> Scaffold {
    let config = format!(
        "# Generated by `burrow init`.\n\
         # `db/seed.sql` runs exactly once, after PostgreSQL is healthy.\n\n\
         [project]\n\
         name = \"{name}\"\n\n\
         [services.db]\n\
         use = \"postgres@16\"\n\
         seed = \"db/seed.sql\"\n"
    );
    Scaffold {
        template: "postgres",
        summary: "PostgreSQL 16".to_owned(),
        config,
        services: vec![ServicePreview {
            name: "db".to_owned(),
            command: "postgres@16".to_owned(),
            route: None,
        }],
        extra_files: vec![(
            PathBuf::from("db").join("seed.sql"),
            "-- Seed data for local development.\n\
             -- Burrow runs this file once, after PostgreSQL is healthy.\n\
             -- Add your schema and data here, for example:\n\
             --\n\
             -- CREATE TABLE todos (\n\
             --     id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,\n\
             --     title text NOT NULL\n\
             -- );\n"
                .to_owned(),
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::{module_name, project_name, run_command};
    use std::path::Path;

    #[test]
    fn project_names_are_sanitized_for_the_schema() {
        assert_eq!(project_name(Path::new("C:\\work\\My Shop")), "my-shop");
        assert_eq!(project_name(Path::new("C:\\work\\__shop__")), "shop");
        assert_eq!(project_name(Path::new("C:\\work\\123")), "123");
        assert_eq!(project_name(Path::new("C:\\")), "project");
    }

    #[test]
    fn node_commands_respect_the_package_manager() {
        assert_eq!(
            run_command("npm", "dev", &["--port", "{{web.port}}"]),
            "npm run dev -- --port {{web.port}}"
        );
        assert_eq!(
            run_command("yarn", "dev", &["--port", "{{web.port}}"]),
            "yarn run dev --port {{web.port}}"
        );
        assert_eq!(run_command("pnpm", "dev", &[]), "pnpm run dev");
    }

    #[test]
    fn python_modules_are_relative_to_the_project_root() {
        let root = Path::new("C:\\proj");
        assert_eq!(
            module_name(Path::new("C:\\proj\\app\\main.py"), root),
            Some("app.main".to_owned())
        );
        assert_eq!(
            module_name(Path::new("C:\\proj\\main.py"), root),
            Some("main".to_owned())
        );
        assert_eq!(
            module_name(Path::new("C:\\proj\\app\\__init__.py"), root),
            Some("app".to_owned())
        );
        assert_eq!(
            module_name(Path::new("C:\\proj\\my-app\\main.py"), root),
            None
        );
    }
}
