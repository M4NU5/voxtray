//! `voxtray-engine`: Kokoro text-to-speech, driven by Voxtray over stdin and
//! stdout with the protocol in [`voxtray_engine::protocol`]. Voxtray keeps
//! one loaded for a while after each reading.
//!
//! ```text
//! voxtray-engine --model-dir <dir> --out-dir <dir> [--voice af_heart] [--threads N]
//! ```
//!
//! `--voice` picks the lexicon (American or British) loaded with the model.
//! Each request still names its own voice.

mod kokoro;

use std::path::PathBuf;

use voxtray_engine::protocol::{self, Response};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut out = std::io::stdout();
    let fail = |out: &mut std::io::Stdout, message: String, code: i32| -> ! {
        let _ = protocol::send(out, &Response::Error { id: None, message });
        std::process::exit(code);
    };

    let (Some(model_dir), Some(out_dir)) = (
        voxtray_engine::arg(&args, "--model-dir"),
        voxtray_engine::arg(&args, "--out-dir"),
    ) else {
        fail(
            &mut out,
            "usage: voxtray-engine --model-dir <dir> --out-dir <dir> [--voice <name>] [--threads <n>]"
                .into(),
            2,
        );
    };
    let voice = voxtray_engine::arg(&args, "--voice").unwrap_or_else(|| "af_heart".into());
    // RTF stops improving past about 8 threads (0.37 on an i7-13700K).
    let threads = voxtray_engine::arg(&args, "--threads")
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(4, |n| n.get().min(8) as i32)
        });

    let started = std::time::Instant::now();
    let mut backend = match kokoro::Kokoro::load(model_dir.as_ref(), &voice, threads) {
        Ok(backend) => backend,
        Err(message) => fail(&mut out, message, 1),
    };
    eprintln!(
        "kokoro loaded in {} ms with {threads} threads",
        started.elapsed().as_millis()
    );

    let stdin = std::io::stdin();
    if let Err(e) = protocol::serve(
        stdin.lock(),
        out.lock(),
        &mut backend,
        &PathBuf::from(out_dir),
    ) {
        eprintln!("engine stopped: {e}");
        std::process::exit(1);
    }
}
