//! Stdin harness for the sanitizer corpus differential.
//!
//! Usage: `sanitize_stdin true|false < input.txt`
//! Reads all of stdin, runs `strip_agent_artifacts`, writes the result to
//! stdout with no trailing newline added (Python `print(..., end="")` parity).

use claw_core::sanitize::strip_agent_artifacts;
use std::env;
use std::io::{self, Read, Write};

fn main() {
    let collapse = match env::args().nth(1).as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => {
            eprintln!("usage: sanitize_stdin true|false < stdin");
            std::process::exit(2);
        }
    };

    let mut buf = Vec::new();
    io::stdin()
        .read_to_end(&mut buf)
        .expect("read stdin");
    let input = String::from_utf8(buf).expect("stdin must be valid UTF-8");
    let out = strip_agent_artifacts(&input, collapse);
    // No trailing newline added — write bytes as-is so cmp is meaningful.
    io::stdout()
        .write_all(out.as_bytes())
        .expect("write stdout");
}
