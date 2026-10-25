#![no_main]

use std::collections::BTreeMap;
use std::path::PathBuf;

use burrow_config::{render_template, ServiceVars, TemplateContext};
use libfuzzer_sys::fuzz_target;

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

// Template text comes from `burrow.toml`. The parser must handle arbitrary
// braces, dots, and Unicode without panicking, and unknown variables must be
// errors rather than empty substitutions.
fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let _ = render_template(input, &context());
    }
});
