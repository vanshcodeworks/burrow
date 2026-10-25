#![no_main]

use burrow_pkg::Registry;
use libfuzzer_sys::fuzz_target;

// Registry entries are embedded today, but `load_directory` accepts external
// TOML. Parsing must reject malformed or unsafe entries without panicking.
fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let mut registry = Registry::default();
        let _ = registry.add_toml(text, "fuzz");
    }
});
