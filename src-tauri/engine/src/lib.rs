//! Voxtray's speech engine protocol, and the host-side client that drives an
//! engine process with it. The engine itself is the `voxtray-engine` binary.

mod client;
pub mod protocol;

pub use client::Client;

/// The value after `--name` in `args`, if present.
pub fn arg(args: &[String], name: &str) -> Option<String> {
    let at = args.iter().position(|a| a == name)?;
    args.get(at + 1).cloned()
}
