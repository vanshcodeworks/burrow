//! Property tests for template parsing and substitution.

use std::collections::BTreeMap;
use std::path::PathBuf;

use burrow_config::{parse, render_template, resolve_templates, ServiceVars, TemplateContext};
use proptest::prelude::*;

fn context() -> TemplateContext {
    let mut services = BTreeMap::new();
    services.insert(
        "db".to_owned(),
        ServiceVars {
            host: "127.0.0.1".to_owned(),
            port: Some(5432),
            url: Some("postgres://burrow@127.0.0.1:5432/app".to_owned()),
            password: Some("generated".to_owned()),
        },
    );
    services.insert(
        "api".to_owned(),
        ServiceVars {
            host: "127.0.0.1".to_owned(),
            port: Some(3000),
            url: Some("http://127.0.0.1:3000".to_owned()),
            password: None,
        },
    );
    TemplateContext {
        project_name: "shop".to_owned(),
        project_dir: PathBuf::from("C:\\shop"),
        services,
    }
}

proptest! {
    #[test]
    fn text_without_markers_is_unchanged(text in "[^{}]{0,64}") {
        prop_assert_eq!(
            render_template(&text, &context()).expect("plain text"),
            text
        );
    }

    #[test]
    fn known_substitution_keeps_surrounding_text(
        prefix in "[^{}]{0,32}",
        suffix in "[^{}]{0,32}",
    ) {
        let input = format!("{prefix}{{{{project.name}}}}{suffix}");
        let expected = format!("{prefix}shop{suffix}");
        prop_assert_eq!(
            render_template(&input, &context()).expect("known variable"),
            expected
        );
    }

    #[test]
    fn an_unclosed_marker_is_an_error(
        prefix in "[^{}]{0,32}",
        tail in "[^{}]{0,32}",
    ) {
        let mut input = prefix;
        input.push_str("{{");
        input.push_str(&tail);
        prop_assert!(render_template(&input, &context()).is_err());
    }

    #[test]
    fn an_unknown_variable_is_an_error(name in "[a-z][a-z0-9_]{0,8}") {
        let input = format!("{{{{{name}}}}}");
        prop_assert!(render_template(&input, &context()).is_err());
    }

    #[test]
    fn resolve_templates_preserves_plain_values(value in "[^{}]{0,64}") {
        let mut config = parse(
            "[project]\nname = \"shop\"\n\n[services.api]\nrun = \"api.exe\"\n",
        )
        .expect("valid config");
        config
            .services
            .get_mut("api")
            .expect("api service")
            .env
            .insert("VALUE".to_owned(), value.clone());

        resolve_templates(&mut config, &context()).expect("resolve");
        prop_assert_eq!(&config.services["api"].env["VALUE"], &value);
    }
}
