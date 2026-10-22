//! A stand-in engine for the protocol tests. It speaks the real protocol but
//! makes 1 ms of tone per character instead of speech. The text steers it:
//! `fail` gets an error reply, `hang` never answers and `crash` exits
//! mid-request.
//!
//! Flags: `--out-dir <dir>` (required), `--fail-load`, `--protocol <n>` (send
//! a ready with that version, then stop), `--noise` (print a non-protocol
//! line first).

use std::io::Write;
use std::path::PathBuf;

use voxtray_engine::protocol::{self, Backend, Ready, Response};

struct Fake;

impl Backend for Fake {
    fn name(&self) -> &str {
        "fake"
    }

    fn sample_rate(&self) -> u32 {
        1000
    }

    fn synthesize(&mut self, text: &str, _voice: &str, _speed: f32) -> Result<Vec<f32>, String> {
        match text {
            "fail" => Err("no voice for that".into()),
            "hang" => loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            },
            "crash" => {
                eprintln!("fake engine crashed");
                std::process::exit(3);
            }
            _ => Ok(vec![0.5; text.chars().count()]),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let mut out = std::io::stdout();

    if has("--noise") {
        println!("hello from native code");
    }
    if has("--fail-load") {
        eprintln!("model.onnx is missing");
        let error = Response::Error {
            id: None,
            message: "could not load the model".into(),
        };
        let _ = protocol::send(&mut out, &error);
        std::process::exit(1);
    }
    if let Some(version) = voxtray_engine::arg(&args, "--protocol") {
        let ready = Ready {
            protocol: version.parse().unwrap(),
            engine: "fake".into(),
            version: "0".into(),
            sample_rate: 1000,
        };
        let _ = protocol::send(&mut out, &Response::Ready(ready));
        let _ = out.flush();
        return;
    }

    let out_dir = PathBuf::from(voxtray_engine::arg(&args, "--out-dir").expect("--out-dir"));
    let stdin = std::io::stdin();
    protocol::serve(stdin.lock(), out.lock(), &mut Fake, &out_dir).unwrap();
}
