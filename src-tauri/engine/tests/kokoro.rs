//! The real engine. Tests that need the model are ignored by default: set
//! `VOXTRAY_KOKORO_MODEL` to an unpacked `kokoro-multi-lang-v1_0` folder and
//! run `cargo test -p voxtray-engine -- --ignored`.

use std::process::Command;

use voxtray_engine::Client;

fn engine(voice: &str) -> Client {
    let model = std::env::var_os("VOXTRAY_KOKORO_MODEL").expect("set VOXTRAY_KOKORO_MODEL");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtray-engine"));
    cmd.arg("--model-dir").arg(model).args(["--voice", voice]);
    Client::spawn(cmd, "Kokoro").expect("spawn voxtray-engine")
}

/// Seconds of audio in a mono 16-bit WAV at 24 kHz.
fn seconds(wav: &[u8]) -> f32 {
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 24_000);
    (wav.len() - 44) as f32 / 2.0 / 24_000.0
}

#[test]
#[ignore = "needs the Kokoro model"]
fn warm_engine_synthesises_chunks_in_order() {
    let mut engine = engine("af_heart");
    assert_eq!(engine.ready().unwrap().engine, "kokoro");
    let short = seconds(&engine.synthesize("Hi.", "af_heart", 1.0).unwrap());
    let long = seconds(
        &engine
            .synthesize(
                "This sentence is a good deal longer than that.",
                "af_heart",
                1.0,
            )
            .unwrap(),
    );
    assert!(long > short, "{long} > {short}");
    // Punctuation alone, or a script the model can't read, is short silence.
    assert!(seconds(&engine.synthesize("...", "af_heart", 1.0).unwrap()) < 0.5);
    assert!(seconds(&engine.synthesize("日本", "af_heart", 1.0).unwrap()) < 0.5);
    // A NUL would break the C API; it is read as a space.
    assert!(engine.synthesize("a\0b", "af_heart", 1.0).is_ok());
}

#[test]
#[ignore = "needs the Kokoro model"]
fn speed_and_voice_apply_per_request() {
    let mut engine = engine("bf_emma");
    let text = "Speed changes how long this takes to say.";
    let normal = seconds(&engine.synthesize(text, "bf_emma", 1.0).unwrap());
    let fast = seconds(&engine.synthesize(text, "bf_emma", 2.0).unwrap());
    assert!(fast < normal * 0.7, "{fast} vs {normal}");
    assert!(engine.synthesize(text, "bm_george", 1.0).is_ok());
    assert_eq!(
        engine.synthesize(text, "nobody", 1.0),
        Err("Kokoro engine error: unknown Kokoro voice nobody".into())
    );
}

#[test]
#[ignore = "needs the Kokoro model"]
fn killing_the_engine_unblocks_synthesis_with_an_error() {
    let mut engine = engine("af_heart");
    engine.ready().unwrap();
    engine.killer()();
    assert!(engine.synthesize("Hello there.", "af_heart", 1.0).is_err());
}

#[test]
fn a_missing_model_is_reported_at_start() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtray-engine"));
    cmd.args(["--model-dir", "/nonexistent"]);
    let mut engine = Client::spawn(cmd, "Kokoro").unwrap();
    let err = engine.ready().err().unwrap();
    assert!(
        err.starts_with("Kokoro engine could not start: could not load the Kokoro model"),
        "{err}"
    );
}
