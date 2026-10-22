//! Piper: free, offline, neural local voice. The engine binary and voice model
//! are fetched on first use (see [`crate::download`]).
//!
//! One Piper process stays warm for a whole reading, so the voice model loads
//! once. Each chunk is one JSON line on stdin naming a WAV file to write; Piper
//! prints that path on stdout when the file is done.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use tauri::AppHandle;

use crate::download;
use crate::settings::Settings;
use crate::timeline::Pcm;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Stderr lines kept for error messages.
const STDERR_TAIL: usize = 8;

/// A running Piper process that synthesises one chunk at a time.
pub struct Engine {
    child: Arc<Mutex<Child>>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    out_dir: PathBuf,
    next: u64,
}

impl Engine {
    /// Download Piper and the selected voice if needed, then start the engine.
    pub fn open(app: &AppHandle, settings: &Settings) -> Result<Engine, String> {
        let exe = download::ensure_engine(app)?;
        let model = download::ensure_voice(app, &settings.piper_voice)?;
        Engine::spawn(&exe, &model, settings.speed)
    }

    pub fn spawn(exe: &Path, model: &Path, speed: f32) -> Result<Engine, String> {
        let engine_dir = exe.parent().ok_or("engine directory not found")?;
        let out_dir = std::env::temp_dir().join(format!(
            "voxtray-piper-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&out_dir).map_err(|e| format!("Piper temp dir: {e}"))?;

        // Piper uses length_scale as the inverse of speed (higher = slower).
        let speed = speed.clamp(0.5, 2.0);
        let length_scale = (1.0 / speed).to_string();

        let mut cmd = Command::new(exe);
        cmd.current_dir(engine_dir);
        cmd.args([
            "--model",
            model.to_string_lossy().as_ref(),
            "--length_scale",
            &length_scale,
            "--json-input",
        ]);

        // Help the dynamic loader find the bundled onnxruntime next to the binary.
        #[cfg(all(unix, not(target_os = "macos")))]
        cmd.env("LD_LIBRARY_PATH", engine_dir);
        #[cfg(target_os = "macos")]
        cmd.env("DYLD_LIBRARY_PATH", engine_dir);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start Piper: {e}"))?;

        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().ok_or("Piper stdout unavailable")?);

        // Piper logs a line per chunk. Drain stderr so the pipe never fills
        // and blocks it, keeping the last few lines for error messages.
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(pipe) = child.stderr.take() {
            let tail = stderr.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                    if t.len() == STDERR_TAIL {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            });
        }

        Ok(Engine {
            child: Arc::new(Mutex::new(child)),
            stdin,
            stdout,
            stderr,
            out_dir,
            next: 0,
        })
    }

    /// Kills the process from another thread, unblocking [`Engine::synthesize`].
    pub fn killer(&self) -> impl FnOnce() + Send + 'static {
        let child = self.child.clone();
        move || {
            let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill();
        }
    }

    pub fn synthesize(&mut self, text: &str) -> Result<Pcm, String> {
        let out = self.out_dir.join(format!("{}.wav", self.next));
        self.next += 1;

        let line = serde_json::json!({ "text": text, "output_file": out });
        let stdin = self.stdin.as_mut().ok_or("Piper stdin closed")?;
        writeln!(stdin, "{line}")
            .and_then(|()| stdin.flush())
            .map_err(|e| self.failed(&format!("write to Piper: {e}")))?;

        let mut done = String::new();
        let n = self
            .stdout
            .read_line(&mut done)
            .map_err(|e| self.failed(&format!("read from Piper: {e}")))?;
        if n == 0 {
            return Err(self.failed("Piper stopped unexpectedly"));
        }

        let mut bytes = Vec::new();
        std::fs::File::open(&out)
            .and_then(|mut f| f.read_to_end(&mut bytes))
            .map_err(|e| format!("read Piper output: {e}"))?;
        let _ = std::fs::remove_file(&out);
        Pcm::decode(bytes)
    }

    fn failed(&self, what: &str) -> String {
        let tail = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        let last = tail.iter().rev().find(|l| !l.contains("[info]"));
        match last {
            Some(line) => format!("{what}: {}", line.trim()),
            None => what.to_string(),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Closing stdin lets Piper exit cleanly; kill in case it is mid-chunk.
        self.stdin.take();
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.out_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real Piper binary and a voice, from `VOXTRAY_PIPER_EXE` and
    /// `VOXTRAY_PIPER_MODEL`. Run with `cargo test -- --ignored`.
    fn engine() -> Engine {
        let exe = std::env::var_os("VOXTRAY_PIPER_EXE").expect("set VOXTRAY_PIPER_EXE");
        let model = std::env::var_os("VOXTRAY_PIPER_MODEL").expect("set VOXTRAY_PIPER_MODEL");
        Engine::spawn(Path::new(&exe), Path::new(&model), 1.0).expect("spawn Piper")
    }

    #[test]
    #[ignore = "needs a Piper binary and voice"]
    fn warm_engine_synthesises_chunks_in_order() {
        let mut engine = engine();
        let short = engine.synthesize("Hi.").unwrap();
        let long = engine.synthesize("This sentence is a good deal longer than that.").unwrap();
        assert_eq!(short.channels, 1);
        assert!(short.sample_rate >= 16_000);
        assert!(long.duration() > short.duration());
        // Punctuation alone still yields a valid (silent) clip.
        assert!(engine.synthesize("...").is_ok());
    }

    #[test]
    #[ignore = "needs a Piper binary and voice"]
    fn killing_the_engine_unblocks_synthesis_with_an_error() {
        let mut engine = engine();
        engine.killer()();
        assert!(engine.synthesize("Hello there.").is_err());
    }
}
