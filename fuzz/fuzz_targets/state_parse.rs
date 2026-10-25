#![no_main]

use libfuzzer_sys::fuzz_target;

// `status`, `down`, and `logs` read `.burrow/state.json` when the control
// socket is unavailable. A truncated or hostile file must produce the
// documented error, not a panic.
fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = burrow_cli::state::RuntimeState::parse(text);
    }
});
