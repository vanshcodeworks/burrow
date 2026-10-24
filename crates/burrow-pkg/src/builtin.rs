//! Registry entries compiled into the Burrow binary.
//!
//! Each entry lives in `registry/<name>.toml` at the repository root and is
//! embedded with `include_str!`, so `burrow.exe` works as a single file. The
//! embedded copies are the source of truth; there is no unverified network
//! registry.

/// `(file name, TOML text)` pairs for every built-in registry entry.
pub const ENTRIES: &[(&str, &str)] = &[(
    "postgres.toml",
    include_str!("../../../registry/postgres.toml"),
)];

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
}
