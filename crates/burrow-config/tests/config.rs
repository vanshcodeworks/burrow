use std::collections::BTreeMap;

use burrow_config::{parse, parse_with_local, Config, PortConfig, RestartPolicy};
use proptest::prelude::*;

fn base() -> String {
    r#"
[project]
name = "shop"

[services.db]
run = "db.exe"

[services.api]
run = "api.exe"
needs = ["db"]
"#
    .to_owned()
}

#[test]
fn parses_a_minimal_project() {
    let config = parse(&base()).expect("valid config");
    assert_eq!(config.project.name, "shop");
    assert_eq!(config.services.len(), 2);
    assert_eq!(config.services["api"].needs, vec!["db"]);
    assert_eq!(config.services["api"].port, PortConfig::Auto);
    assert_eq!(config.services["api"].restart, RestartPolicy::OnFailure);
}

#[test]
fn rejects_invalid_project_names() {
    let error = parse(
        r#"
[project]
name = "Shop_App"
[services.api]
run = "api.exe"
"#,
    )
    .expect_err("must fail");
    assert!(error.to_string().contains("project.name"), "{error}");
}

#[test]
fn rejects_reserved_windows_device_names() {
    for name in ["con", "nul", "com1", "lpt9"] {
        let error = parse(&format!(
            r#"
[project]
name = "shop"
[services.{name}]
run = "x.exe"
"#
        ))
        .expect_err("reserved service name must fail");
        assert!(
            error.to_string().contains("reserved Windows device name"),
            "{name}: {error}"
        );
    }

    let error = parse(
        r#"
[project]
name = "con"
[services.api]
run = "api.exe"
"#,
    )
    .expect_err("reserved project name must fail");
    assert!(
        error.to_string().contains("reserved Windows device name"),
        "{error}"
    );
}

#[test]
fn requires_exactly_one_of_use_and_run() {
    let error = parse(
        r#"
[project]
name = "shop"
[services.api]
"#,
    )
    .expect_err("must fail");
    assert!(error.to_string().contains("run"), "{error}");

    let error = parse(
        r#"
[project]
name = "shop"
[services.api]
use = "node@22"
run = "node"
"#,
    )
    .expect_err("must fail");
    assert!(error.to_string().contains("mutually exclusive"), "{error}");
}

#[test]
fn suggests_a_dependency_typo() {
    let error = parse(
        r#"
[project]
name = "shop"
[services.db]
run = "db.exe"
[services.api]
run = "api.exe"
needs = ["dbb"]
"#,
    )
    .expect_err("must fail");
    let message = error.to_string();
    assert!(message.contains("unknown service `dbb`"), "{message}");
    assert!(message.contains("did you mean `db`?"), "{message}");
}

#[test]
fn reports_the_cycle_path() {
    let error = parse(
        r#"
[project]
name = "shop"
[services.a]
run = "a.exe"
needs = ["c"]
[services.b]
run = "b.exe"
needs = ["a"]
[services.c]
run = "c.exe"
needs = ["b"]
"#,
    )
    .expect_err("must fail");
    let message = error.to_string();
    assert!(message.contains("dependency cycle"), "{message}");
    assert!(message.contains("a -> c -> b -> a"), "{message}");
}

#[test]
fn local_overrides_merge_over_the_base() {
    let local = r#"
[services.api]
memory = "512MB"
[services.api.env]
DEBUG = "1"
"#;
    let config = parse_with_local(&base(), Some(local), "burrow.toml").expect("merged config");
    assert_eq!(config.services["api"].run.as_deref(), Some("api.exe"));
    assert_eq!(config.services["api"].memory.as_deref(), Some("512MB"));
    assert_eq!(config.services["api"].env["DEBUG"], "1");
}

#[test]
fn container_references_are_validated() {
    let config = parse(
        r#"
[project]
name = "shop"
[services.cache]
use = "container:redis:7-alpine"
"#,
    )
    .expect("container references are valid");
    assert_eq!(
        config.services["cache"].container_image(),
        Some("redis:7-alpine")
    );
    assert!(!config.services["cache"].is_registry());
    assert!(config.validate().is_ok());

    for bad in ["container:", "container:re dis"] {
        let error = parse(&format!(
            r#"
[project]
name = "shop"
[services.cache]
use = "{bad}"
"#
        ))
        .expect_err("must fail");
        assert!(error.to_string().contains("container"), "{error}");
    }
}

#[test]
fn container_port_and_seed_are_rejected_on_non_container_services() {
    let error = parse(
        r#"
[project]
name = "shop"
[services.cache]
run = "cache.exe"
container_port = 6379
"#,
    )
    .expect_err("must fail");
    assert!(error.to_string().contains("container_port"), "{error}");

    let error = parse(
        r#"
[project]
name = "shop"
[services.cache]
use = "container:redis:7"
seed = "cache/seed.sql"
"#,
    )
    .expect_err("must fail");
    assert!(error.to_string().contains("seed"), "{error}");
}

#[test]
fn watch_paths_stay_inside_the_project() {
    let config = parse(
        r#"
[project]
name = "shop"
[services.api]
run = "api.exe"
watch = ["api/src"]
"#,
    )
    .expect("relative watch paths are valid");
    assert_eq!(config.services["api"].watch, vec!["api/src"]);

    for bad in ["../outside", "C:/elsewhere"] {
        let error = parse(&format!(
            r#"
[project]
name = "shop"
[services.api]
run = "api.exe"
watch = ["{bad}"]
"#
        ))
        .expect_err("must fail");
        assert!(error.to_string().contains("watch"), "{error}");
    }
}

#[test]
fn idle_stop_requires_a_positive_duration() {
    let error = parse(
        r#"
[project]
name = "shop"
[services.api]
run = "api.exe"
idle_stop = "0s"
"#,
    )
    .expect_err("must fail");
    assert!(error.to_string().contains("idle_stop"), "{error}");
}

#[test]
fn parses_ports_health_and_limits() {
    let config = parse(
        r#"
[project]
name = "shop"
[services.api]
run = "api.exe"
port = 3000
memory = "256MB"
cpu = "50%"
health = { http = "/health", timeout = "10s", interval = "200ms" }
restart = "always"
"#,
    )
    .expect("valid config");
    let service = &config.services["api"];
    assert_eq!(service.port, PortConfig::Fixed(3000));
    assert_eq!(
        service.memory_bytes().expect("memory"),
        Some(256 * 1024 * 1024)
    );
    assert_eq!(service.cpu_percent().expect("cpu"), Some(50));
    assert_eq!(service.restart, RestartPolicy::Always);
    assert_eq!(
        service.health_kind(),
        Some(burrow_config::HealthKind::Http("/health".to_owned()))
    );
}

#[test]
fn rejects_bad_health_checks() {
    for health in [
        "{}",
        "{ http = \"/health\", tcp = true }",
        "{ tcp = false }",
        "{ http = \"health\" }",
    ] {
        let text = format!(
            r#"
[project]
name = "shop"
[services.api]
run = "api.exe"
health = {health}
"#
        );
        assert!(parse(&text).is_err(), "health {health} should be rejected");
    }
}

#[test]
fn startup_plan_groups_independent_services() {
    let config = parse(
        r#"
[project]
name = "shop"
[services.db]
run = "db.exe"
[services.cache]
run = "cache.exe"
[services.api]
run = "api.exe"
needs = ["db", "cache"]
[services.web]
run = "web.exe"
needs = ["api"]
"#,
    )
    .expect("valid config");

    let plan = config.startup_plan(&[]).expect("plan");
    assert_eq!(plan.layers.len(), 3);
    assert_eq!(plan.layers[0], vec!["cache", "db"]);
    assert_eq!(plan.layers[1], vec!["api"]);
    assert_eq!(plan.layers[2], vec!["web"]);
    assert_eq!(plan.order(), vec!["cache", "db", "api", "web"]);

    let plan = config
        .startup_plan(&["api".to_owned()])
        .expect("selected plan");
    assert_eq!(plan.layers[0], vec!["cache", "db"]);
    assert_eq!(plan.layers[1], vec!["api"]);
    assert_eq!(plan.included.len(), 3);
}

#[test]
fn dependents_are_transitive() {
    let config = parse(&base()).expect("valid config");
    assert!(config.dependents_of("db").contains("api"));
    assert!(config.dependents_of("api").is_empty());
}

#[test]
fn executable_hash_tracks_commands_but_not_limits() {
    let config = parse(&base()).expect("valid config");
    let same = parse(&base()).expect("valid");
    assert_eq!(config.executable_hash(), same.executable_hash());

    let with_limit =
        parse(&base().replace("run = \"api.exe\"", "run = \"api.exe\"\nmemory = \"512MB\""))
            .expect("valid");
    assert_eq!(config.executable_hash(), with_limit.executable_hash());

    let changed = parse(&base().replace("run = \"api.exe\"", "run = \"api2.exe\"")).expect("valid");
    assert_ne!(config.executable_hash(), changed.executable_hash());
}

#[test]
fn byte_sizes_and_cpu_percentages_are_strict() {
    assert_eq!(burrow_config::parse_byte_size("256MB").unwrap(), 256 << 20);
    assert_eq!(burrow_config::parse_byte_size("1 GiB").unwrap(), 1 << 30);
    assert!(burrow_config::parse_byte_size("256").is_err());
    assert!(burrow_config::parse_byte_size("0MB").is_err());
    assert_eq!(burrow_config::parse_cpu_percent("50%").unwrap(), 50);
    assert!(burrow_config::parse_cpu_percent("0%").is_err());
    assert!(burrow_config::parse_cpu_percent("50").is_err());
}

#[test]
fn schema_mentions_the_main_fields() {
    let schema = burrow_config::json_schema();
    for needle in [
        "project", "services", "run", "use", "auto", "memory", "health",
    ] {
        assert!(schema.contains(needle), "schema is missing `{needle}`");
    }
}

#[test]
fn route_and_seed_paths_are_validated() {
    let bad_route = parse(
        r#"
[project]
name = "shop"
[services.web]
run = "web.exe"
route = "not-a-host"
"#,
    );
    assert!(bad_route.is_err());

    let escape = parse(
        r#"
[project]
name = "shop"
[services.db]
run = "db.exe"
seed = "../seed.sql"
"#,
    );
    assert!(escape.is_err());
}

proptest! {
    #[test]
    fn random_dependency_graphs_produce_a_complete_plan(edges in prop::collection::vec((0u8..8, 0u8..8), 0..24)) {
        // Build only edges from a lower index to a higher index: guaranteed acyclic.
        let mut text = String::from("[project]\nname = \"shop\"\n");
        for node in 0..8u8 {
            text.push_str(&format!("[services.s{node}]\nrun = \"s{node}.exe\"\n"));
            let needs: std::collections::BTreeSet<String> = edges
                .iter()
                .filter(|(from, to)| *to == node && from < to)
                .map(|(from, _)| format!("\"s{from}\""))
                .collect();
            let needs: Vec<String> = needs.into_iter().collect();
            if !needs.is_empty() {
                text.push_str(&format!("needs = [{}]\n", needs.join(", ")));
            }
        }

        let config = parse(&text).expect("generated config is valid");
        let plan = config.startup_plan(&[]).expect("generated graph is acyclic");
        prop_assert_eq!(plan.order().len(), 8);
        // Every dependency must appear in an earlier layer.
        let mut seen = std::collections::BTreeSet::new();
        for layer in &plan.layers {
            for name in layer {
                for need in &config.services[name].needs {
                    prop_assert!(seen.contains(need));
                }
            }
            seen.extend(layer.iter().cloned());
        }
    }

    #[test]
    fn arbitrary_sizes_never_panic(raw in "\\PC{0,24}") {
        let _ = burrow_config::parse_byte_size(&raw);
    }

    #[test]
    fn every_service_gets_a_config_entry(names in prop::collection::btree_set("[a-z][a-z0-9]{0,7}", 1..6)) {
        let mut text = String::from("[project]\nname = \"shop\"\n");
        for name in &names {
            text.push_str(&format!("[services.{name}]\nrun = \"app.exe\"\n"));
        }
        let config: Config = parse(&text).expect("valid config");
        let parsed: BTreeMap<String, ()> = config.services.keys().map(|name| (name.clone(), ())).collect();
        prop_assert_eq!(parsed.len(), names.len());
    }
}
