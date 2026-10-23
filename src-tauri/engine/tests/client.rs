//! The client against a real process: the fake engine, which speaks the
//! protocol without a model.

use std::process::Command;
use std::time::{Duration, Instant};

use voxtray_engine::Client;

fn fake(name: &str, flags: &[&str]) -> Client {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fake-engine"));
    cmd.args(flags);
    Client::spawn(cmd, name).expect("spawn the fake engine")
}

/// Sample count of a mono 16-bit WAV.
fn samples(wav: &[u8]) -> usize {
    assert_eq!(&wav[..4], b"RIFF");
    (wav.len() - 44) / 2
}

#[test]
fn answers_requests_in_order_with_wav_audio() {
    let mut engine = fake("Fake", &[]);
    let ready = engine.ready().unwrap();
    assert_eq!((ready.engine.as_str(), ready.sample_rate), ("fake", 1000));
    assert_eq!(samples(&engine.synthesize("abcd", "v", 1.0).unwrap()), 4);
    assert_eq!(samples(&engine.synthesize("ab", "v", 1.0).unwrap()), 2);
}

#[test]
fn an_error_reply_fails_one_request_only() {
    let mut engine = fake("Fake", &[]);
    assert_eq!(
        engine.synthesize("fail", "v", 1.0),
        Err("Fake engine error: no voice for that".into())
    );
    assert_eq!(samples(&engine.synthesize("abc", "v", 1.0).unwrap()), 3);
}

#[test]
fn a_crash_mid_request_reports_the_last_stderr_line() {
    let mut engine = fake("Fake", &[]);
    assert_eq!(
        engine.synthesize("crash", "v", 1.0),
        Err("Fake engine stopped unexpectedly: fake engine crashed".into())
    );
    assert!(!engine.is_running());
}

#[test]
fn an_engine_keeps_running_between_requests_until_killed() {
    let mut engine = fake("Fake", &[]);
    engine.synthesize("abc", "v", 1.0).unwrap();
    assert!(engine.is_running());
    engine.killer()();
    let deadline = Instant::now() + Duration::from_secs(5);
    while engine.is_running() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!engine.is_running());
}

#[test]
fn the_killer_unblocks_a_request_in_progress() {
    let mut engine = fake("Fake", &[]);
    let kill = engine.killer();
    let started = Instant::now();
    let killer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        kill();
    });
    let result = engine.synthesize("hang", "v", 1.0);
    killer.join().unwrap();
    assert!(
        matches!(&result, Err(e) if e.starts_with("Fake engine stopped unexpectedly")),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_failed_load_is_reported_with_stderr() {
    let mut engine = fake("Fake", &["--fail-load"]);
    assert_eq!(
        engine.synthesize("abc", "v", 1.0),
        Err("Fake engine could not start: could not load the model: model.onnx is missing".into())
    );
}

#[test]
fn another_protocol_version_is_refused() {
    let mut engine = fake("Fake", &["--protocol", "99"]);
    assert_eq!(
        engine.ready().map(|r| r.protocol),
        Err("Fake engine speaks protocol 99, expected 1".into())
    );
}

#[test]
fn stray_lines_on_stdout_are_skipped() {
    let mut engine = fake("Fake", &["--noise"]);
    assert_eq!(samples(&engine.synthesize("ab", "v", 1.0).unwrap()), 2);
}

#[test]
fn a_missing_engine_fails_to_start() {
    let cmd = Command::new("voxtray-engine-that-does-not-exist");
    let err = Client::spawn(cmd, "Gone").err().unwrap();
    assert!(err.starts_with("failed to start the Gone engine"), "{err}");
}

#[test]
fn dropping_the_client_removes_its_audio_folder() {
    let prefix = format!("voxtray-fakedrop-{}-", std::process::id());
    let folders = || {
        std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .count()
    };
    let mut engine = fake("FakeDrop", &[]);
    engine.synthesize("abc", "v", 1.0).unwrap();
    assert_eq!(folders(), 1);
    drop(engine);
    assert_eq!(folders(), 0);
}
