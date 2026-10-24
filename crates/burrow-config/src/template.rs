//! Template expansion for service commands and environment variables.
//!
//! Supported variables:
//!
//! * `{{project.name}}`
//! * `{{project.dir}}`
//! * `{{service.url}}`, `{{service.host}}`, `{{service.port}}`,
//!   `{{service.password}}` where `service` is a service name
//! * `{{service.<name>.<field>}}` as an explicit long form
//!
//! Unknown variables are errors. Empty substitutions are never produced for a
//! known variable whose value is unavailable.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::{suggest, ConfigError};
use crate::model::Config;

/// Values available to templates for one service.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceVars {
    /// Host used in generated URLs, usually `127.0.0.1`.
    pub host: String,
    /// Allocated port, if the service listens on one.
    pub port: Option<u16>,
    /// Fully resolved connection URL, if the service has one.
    pub url: Option<String>,
    /// Generated credential, if the service needs one.
    pub password: Option<String>,
}

/// All values available while resolving a configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateContext {
    /// Project name.
    pub project_name: String,
    /// Absolute project directory.
    pub project_dir: PathBuf,
    /// Per-service values keyed by service name.
    pub services: BTreeMap<String, ServiceVars>,
}

/// Expands templates in every user-visible string field.
pub fn resolve_templates(
    config: &mut Config,
    context: &TemplateContext,
) -> Result<(), ConfigError> {
    for (name, service) in &mut config.services {
        let base = format!("services.{name}");
        if let Some(run) = &service.run {
            service.run = Some(render(run, context, &format!("{base}.run"))?);
        }
        if let Some(cwd) = &service.cwd {
            let rendered = render(&cwd.to_string_lossy(), context, &format!("{base}.cwd"))?;
            service.cwd = Some(PathBuf::from(rendered));
        }
        for (key, value) in &mut service.env {
            *value = render(value, context, &format!("{base}.env.{key}"))?;
        }
        if let Some(health) = &mut service.health {
            if let Some(http) = &health.http {
                health.http = Some(render(http, context, &format!("{base}.health.http"))?);
            }
            if let Some(command) = &health.cmd {
                health.cmd = Some(render(command, context, &format!("{base}.health.cmd"))?);
            }
        }
    }
    Ok(())
}

fn render(input: &str, context: &TemplateContext, path: &str) -> Result<String, ConfigError> {
    let mut output = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let after_open = &remaining[start + 2..];
        let Some(end) = after_open.find("}}") else {
            return Err(ConfigError::invalid(
                path,
                "unclosed `{{` in template; add the matching `}}`",
            ));
        };
        let variable = after_open[..end].trim();
        if variable.is_empty() {
            return Err(ConfigError::invalid(path, "empty template variable"));
        }
        output.push_str(&lookup(variable, context, path)?);
        remaining = &after_open[end + 2..];
    }

    if remaining.contains("}}") {
        return Err(ConfigError::invalid(
            path,
            "unexpected `}}` without a matching `{{`",
        ));
    }
    output.push_str(remaining);
    Ok(output)
}

fn lookup(variable: &str, context: &TemplateContext, path: &str) -> Result<String, ConfigError> {
    match variable {
        "project.name" => return Ok(context.project_name.clone()),
        "project.dir" => {
            return Ok(context.project_dir.to_string_lossy().into_owned());
        }
        _ => {}
    }

    let parts: Vec<&str> = variable.split('.').collect();
    let (service_name, field) = match parts.as_slice() {
        [service, field] => (*service, *field),
        ["service", service, field] => (*service, *field),
        _ => {
            return Err(ConfigError::invalid(
                path,
                format!(
                    "unknown template variable `{{{{{variable}}}}}`; use `{{{{project.name}}}}`, \
                     `{{{{project.dir}}}}`, or `{{{{<service>.url}}}}`"
                ),
            ))
        }
    };

    let Some(vars) = context.services.get(service_name) else {
        let suggestion = suggest(service_name, context.services.keys())
            .map(|candidate| format!("; did you mean `{candidate}`?"))
            .unwrap_or_default();
        return Err(ConfigError::invalid(
            path,
            format!("unknown service `{service_name}` in template{suggestion}"),
        ));
    };

    match field {
        "host" => Ok(vars.host.clone()),
        "port" => vars.port.map(|port| port.to_string()).ok_or_else(|| {
            ConfigError::invalid(
                path,
                format!("service `{service_name}` has no port to substitute"),
            )
        }),
        "url" => vars.url.clone().ok_or_else(|| {
            ConfigError::invalid(
                path,
                format!("service `{service_name}` has no URL to substitute"),
            )
        }),
        "password" => vars.password.clone().ok_or_else(|| {
            ConfigError::invalid(
                path,
                format!("service `{service_name}` has no generated password"),
            )
        }),
        other => {
            let suggestion = suggest(
                other,
                ["host", "port", "url", "password"]
                    .iter()
                    .map(|field| (*field).to_owned())
                    .collect::<Vec<String>>()
                    .iter(),
            )
            .map(|candidate| format!("; did you mean `{candidate}`?"))
            .unwrap_or_default();
            Err(ConfigError::invalid(
                path,
                format!(
                    "unknown template field `{other}` for service `{service_name}`{suggestion}"
                ),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_templates, ServiceVars, TemplateContext};
    use crate::parse;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn config(env: &str) -> crate::Config {
        parse(&format!(
            r#"
            [project]
            name = "shop"

            [services.db]
            run = "db.exe"

            [services.api]
            run = "api.exe"
            needs = ["db"]
            env = {{ DATABASE_URL = "{env}" }}
            "#
        ))
        .expect("valid config")
    }

    fn context() -> TemplateContext {
        TemplateContext {
            project_name: "shop".to_owned(),
            project_dir: PathBuf::from(r"C:\work\shop"),
            services: BTreeMap::from([(
                "db".to_owned(),
                ServiceVars {
                    host: "127.0.0.1".to_owned(),
                    port: Some(5432),
                    url: Some("postgres://127.0.0.1:5432".to_owned()),
                    password: Some("secret".to_owned()),
                },
            )]),
        }
    }

    #[test]
    fn resolves_short_and_long_forms() {
        let mut config = config("{{db.url}}|{{service.db.port}}|{{project.name}}|{{project.dir}}");
        resolve_templates(&mut config, &context()).expect("templates resolve");
        assert_eq!(
            config.services["api"].env["DATABASE_URL"],
            r"postgres://127.0.0.1:5432|5432|shop|C:\work\shop"
        );
    }

    #[test]
    fn resolves_password_only_when_present() {
        let mut config = config("{{db.password}}");
        resolve_templates(&mut config, &context()).expect("password resolves");
        assert_eq!(config.services["api"].env["DATABASE_URL"], "secret");
    }

    #[test]
    fn unknown_variable_is_an_error() {
        let mut config = config("{{db.hostname}}");
        let error = resolve_templates(&mut config, &context()).expect_err("must fail");
        let message = error.to_string();
        assert!(message.contains("unknown template field"), "{message}");
        assert!(
            message.contains("services.api.env.DATABASE_URL"),
            "{message}"
        );
    }

    #[test]
    fn unknown_service_suggests_a_near_miss() {
        let mut config = config("{{dbb.url}}");
        let error = resolve_templates(&mut config, &context()).expect_err("must fail");
        assert!(error.to_string().contains("did you mean `db`?"), "{error}");
    }

    #[test]
    fn unclosed_braces_are_rejected() {
        let mut config = config("{{db.url");
        let error = resolve_templates(&mut config, &context()).expect_err("must fail");
        assert!(error.to_string().contains("unclosed"), "{error}");
    }
}
