//! Loading, validating, and templating `burrow.toml`.
//!
//! The crate is intentionally independent of the CLI and the supervisor: it
//! produces a validated, strongly typed [`Config`] that the runtime consumes.
//! Milestone M2 implements parsing, local overrides, dependency graphs,
//! suggestions, and templates.

#![warn(missing_docs)]

mod error;
mod graph;
mod merge;
mod model;
mod template;

pub use error::ConfigError;
pub use graph::StartupPlan;
pub use model::{
    parse_byte_size, parse_cpu_percent, parse_duration, Config, HealthConfig, HealthKind,
    PortConfig, Project, RestartPolicy, Service,
};
pub use template::{resolve_templates, ServiceVars, TemplateContext};

use std::path::Path;

/// The canonical configuration file name.
pub const CONFIG_FILE: &str = "burrow.toml";
/// Per-developer override file, merged over the base configuration.
pub const LOCAL_CONFIG_FILE: &str = "burrow.local.toml";

/// Loads `burrow.toml` (and optional `burrow.local.toml`) from `project_dir`.
pub fn load(project_dir: &Path) -> Result<Config, ConfigError> {
    let base_path = project_dir.join(CONFIG_FILE);
    let base = std::fs::read_to_string(&base_path).map_err(|source| ConfigError::Io {
        path: base_path.clone(),
        source,
    })?;

    let local_path = project_dir.join(LOCAL_CONFIG_FILE);
    let local = match std::fs::read_to_string(&local_path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(ConfigError::Io {
                path: local_path,
                source,
            })
        }
    };

    parse_with_local(&base, local.as_deref(), CONFIG_FILE)
}

/// Parses a base document plus an optional local override document.
///
/// This is the testable core of [`load`]. `source_name` is used in parse
/// errors.
pub fn parse_with_local(
    base: &str,
    local: Option<&str>,
    source_name: &str,
) -> Result<Config, ConfigError> {
    let mut document: toml::Value = toml::from_str(base).map_err(|error| ConfigError::Parse {
        file: source_name.to_owned(),
        message: error.to_string(),
    })?;

    if let Some(local) = local {
        let overlay: toml::Value = toml::from_str(local).map_err(|error| ConfigError::Parse {
            file: LOCAL_CONFIG_FILE.to_owned(),
            message: error.to_string(),
        })?;
        merge::merge_values(&mut document, overlay);
    }

    let config: Config =
        document
            .try_into()
            .map_err(|error: toml::de::Error| ConfigError::Parse {
                file: source_name.to_owned(),
                message: error.to_string(),
            })?;

    config.validate()?;
    Ok(config)
}

/// Parses a single document without local overrides.
pub fn parse(input: &str) -> Result<Config, ConfigError> {
    parse_with_local(input, None, CONFIG_FILE)
}

/// Returns the JSON Schema for `burrow.toml` as a pretty-printed string.
pub fn json_schema() -> String {
    model::schema()
}
