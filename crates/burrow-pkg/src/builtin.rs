//! Registry entries compiled into the Burrow binary.
//!
//! Each entry lives in `registry/<name>.toml` at the repository root and is
//! embedded with `include_str!`, so `burrow.exe` works as a single file. The
//! embedded copies are the source of truth; there is no unverified network
//! registry.

/// `(file name, TOML text)` pairs for every built-in registry entry.
pub const ENTRIES: &[(&str, &str)] = &[
    (
        "postgres.toml",
        include_str!("../../../registry/postgres.toml"),
    ),
    ("nats.toml", include_str!("../../../registry/nats.toml")),
    ("caddy.toml", include_str!("../../../registry/caddy.toml")),
];

#[cfg(test)]
mod tests {
    use crate::registry::Registry;

    #[test]
    fn every_embedded_entry_parses_and_has_a_verifiable_postgres_build() {
        let registry = Registry::builtin().expect("built-in registry parses");
        let postgres = registry.entry("postgres").expect("postgres is embedded");
        assert_eq!(postgres.license, "PostgreSQL");
        assert!(postgres.source.starts_with("https://www.enterprisedb.com/"));

        let resolved = registry
            .resolve("postgres@16")
            .expect("postgres@16 resolves");
        assert_eq!(resolved.version.version, "16.15");
        assert_eq!(resolved.platform.size, 373_254_386);
        assert_eq!(
            resolved.platform.sha256,
            "43bb45f173a6f08cf1d29a97a6d8deb119e8e8093a24c00d2d1001a0ccaa8281"
        );
        assert!(resolved
            .platform
            .url
            .ends_with("/postgresql-16.15-5-windows-x64-binaries.zip"));
        assert!(resolved
            .version
            .init
            .iter()
            .any(|step| step.command.program == "initdb"));
        assert!(resolved.version.seed.is_some());
        assert_eq!(
            resolved.version.health.as_ref().map(|health| health.tcp),
            Some(true)
        );
    }

    #[test]
    fn nats_entry_pins_the_official_windows_build() {
        let registry = Registry::builtin().expect("built-in registry parses");
        let resolved = registry
            .resolve("nats@2.15.1")
            .expect("nats@2.15.1 resolves");
        assert_eq!(resolved.version.version, "2.15.1");
        assert_eq!(resolved.platform.size, 7_484_670);
        assert_eq!(
            resolved.platform.sha256,
            "4f7edfa376750b2720463bd22766ed469a5f13f378a643097866fcd953cbf7e3"
        );
        assert!(resolved
            .platform
            .url
            .ends_with("/nats-server-v2.15.1-windows-amd64.zip"));
        assert_eq!(
            resolved
                .platform
                .binaries
                .get("nats-server")
                .map(String::as_str),
            Some("nats-server-v2.15.1-windows-amd64/nats-server.exe")
        );
        let run = resolved
            .version
            .run
            .as_ref()
            .expect("nats has a run command");
        assert!(run.args.contains(&"127.0.0.1".to_owned()));
        assert_eq!(
            resolved.version.health.as_ref().map(|health| health.tcp),
            Some(true)
        );
    }

    #[test]
    fn caddy_entry_pins_the_official_windows_build() {
        let registry = Registry::builtin().expect("built-in registry parses");
        let resolved = registry
            .resolve("caddy@2.11.7")
            .expect("caddy@2.11.7 resolves");
        assert_eq!(resolved.platform.size, 18_479_498);
        assert_eq!(
            resolved.platform.sha256,
            "0a1edc0b799512051c57071ce0e798d3f2cf67dc98171366f1d2b326072e3b06"
        );
        assert!(resolved
            .platform
            .url
            .ends_with("/caddy_2.11.7_windows_amd64.zip"));
        assert_eq!(
            resolved.platform.binaries.get("caddy").map(String::as_str),
            Some("caddy.exe")
        );
        let run = resolved
            .version
            .run
            .as_ref()
            .expect("caddy has a run command");
        assert!(run.args.iter().any(|argument| argument == "--browse"));
        assert_eq!(
            resolved
                .version
                .health
                .as_ref()
                .and_then(|health| health.http.as_deref()),
            Some("/")
        );
    }
}
