#![no_main]

use std::path::Component;

use libfuzzer_sys::fuzz_target;

// Archive entry names are attacker-controlled. Any name the extractor accepts
// must stay relative and contain only ordinary components; otherwise a zip or
// tar could write outside the destination.
fuzz_target!(|data: &[u8]| {
    let Ok(name) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(path) = burrow_pkg::extract::validate_entry_path(name) {
        assert!(!path.is_absolute(), "accepted an absolute path: {name:?}");
        assert!(
            path.components()
                .all(|component| matches!(component, Component::Normal(_))),
            "accepted a non-normal component: {name:?}"
        );
        assert!(
            !path.as_os_str().is_empty(),
            "accepted an empty path: {name:?}"
        );
    }
});
