#![no_main]

use libfuzzer_sys::fuzz_target;

// The configuration parser is the first thing that reads a user file, so a
// panic here would turn a typo into a crash. Invalid input must return an
// error, never abort.
fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = burrow_config::parse(text);
    }
});
