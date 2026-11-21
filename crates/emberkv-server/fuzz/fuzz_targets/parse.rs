//! Fuzz target: feed arbitrary bytes to the request parser, command after command, exactly as a
//! connection does. It must never panic, never report using more bytes than it was given, and never
//! loop without making progress.

#![no_main]

use emberkv_server::resp::{Parsed, ProtocolLimits, parse};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = ProtocolLimits {
        args: 16,
        arg_len: 4096,
        inline_len: 4096,
    };
    let mut rest = data;
    while let Ok(Parsed::Command { args, consumed }) = parse(rest, &limits) {
        assert!(consumed > 0, "a command must use at least one byte, or the server would spin");
        assert!(consumed <= rest.len(), "used more bytes than were given");
        assert!(args.len() <= limits.args);
        assert!(args.iter().all(|arg| arg.len() <= limits.arg_len.max(limits.inline_len)));
        rest = &rest[consumed..];
    }
});
