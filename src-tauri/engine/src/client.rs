//! The host side: runs an engine process and talks to it over stdio.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::protocol::{Ready, Request, Response, PROTOCOL};

/// Stderr lines kept for error messages.
const STDERR_TAIL: usize = 8;

/// A running engine. Requests go one at a time; [`Client::killer`] aborts
/// one in progress from another thread.
pub struct Client {
    /// Shown in error messages, e.g. `Kokoro`.
    name: String,
    child: Arc<Mutex<Child>>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    stderr_reader: Option<JoinHandle<()>>,
    out_dir: PathBuf,
    ready: Option<Ready>,
    started: Instant,
    next: u64,
}

impl Client {
    /// Start the engine `cmd` with a fresh folder for its audio, passed as
    /// `--out-dir`. Returns at once: the first request waits for the model
    /// to load, so a stop during loading can kill it.
    pub fn spawn(mut cmd: Command, name: &str) -> Result<Client, String> {
        let out_dir = std::env::temp_dir().join(format!(
            "voxtray-{}-{}-{}",
            name.to_lowercase(),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&out_dir).map_err(|e| format!("{name} temp dir: {e}"))?;

        let spawned = cmd
            .arg("--out-dir")
            .arg(&out_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&out_dir);
                return Err(format!("failed to start the {name} engine: {e}"));
            }
        };

        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().ok_or("engine stdout unavailable")?);

        // Drain stderr so the pipe never fills and blocks the engine, keeping
        // the last few lines for error messages.
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_reader = child.stderr.take().map(|pipe| {
            let tail = stderr.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                    if t.len() == STDERR_TAIL {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            })
        });

        Ok(Client {
            name: name.to_string(),
            child: Arc::new(Mutex::new(child)),
            stdin,
            stdout,
            stderr,
            stderr_reader,
            out_dir,
            ready: None,
            started: Instant::now(),
            next: 1,
        })
    }

    /// Kills the engine from another thread, unblocking [`Client::synthesize`].
    pub fn killer(&self) -> impl FnOnce() + Send + 'static {
        let child = self.child.clone();
        move || {
            let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill();
        }
    }

    /// Wait for the engine to load, once. Checks it speaks our protocol.
    pub fn ready(&mut self) -> Result<&Ready, String> {
        if self.ready.is_none() {
            let name = self.name.clone();
            match self.next_response()? {
                Response::Ready(r) if r.protocol == PROTOCOL => {
                    log::info!(
                        "{name} engine {} ready in {} ms",
                        r.version,
                        self.started.elapsed().as_millis()
                    );
                    self.ready = Some(r);
                }
                Response::Ready(r) => {
                    return Err(format!(
                        "{name} engine speaks protocol {}, expected {PROTOCOL}",
                        r.protocol
                    ))
                }
                Response::Error { message, .. } => {
                    let what = format!("{name} engine could not start: {message}");
                    self.reap();
                    return Err(self.failed(&what));
                }
                Response::Result { .. } => {
                    return Err(format!("{name} engine sent a result before it was ready"))
                }
            }
        }
        Ok(self.ready.as_ref().expect("set above"))
    }

    /// Speak `text` and return the audio as WAV bytes.
    pub fn synthesize(&mut self, text: &str, voice: &str, speed: f32) -> Result<Vec<u8>, String> {
        self.ready()?;
        let id = self.next;
        self.next += 1;
        let request = Request::Synthesize {
            id,
            text: text.to_string(),
            voice: voice.to_string(),
            speed,
        };
        let line = serde_json::to_string(&request).map_err(|e| e.to_string())?;
        let stdin = self.stdin.as_mut().ok_or("engine stdin closed")?;
        let written = writeln!(stdin, "{line}").and_then(|()| stdin.flush());
        if let Err(e) = written {
            return Err(self.failed(&format!("write to the {} engine: {e}", self.name)));
        }

        let name = self.name.clone();
        match self.next_response()? {
            Response::Result { id: got, path } if got == id => {
                // Only read files the engine was told to write to.
                if !path.starts_with(&self.out_dir) {
                    return Err(format!("{name} engine wrote audio outside its folder"));
                }
                let bytes = std::fs::read(&path).map_err(|e| format!("read {name} audio: {e}"));
                let _ = std::fs::remove_file(&path);
                bytes
            }
            Response::Error { id: got, message } if got.is_none() || got == Some(id) => {
                Err(format!("{name} engine error: {message}"))
            }
            other => Err(format!("{name} engine answered out of turn: {other:?}")),
        }
    }

    /// The next protocol line. Skips anything else on stdout, such as a
    /// stray print from native code.
    fn next_response(&mut self) -> Result<Response, String> {
        loop {
            let mut line = String::new();
            let n = match self.stdout.read_line(&mut line) {
                Ok(n) => n,
                Err(e) => {
                    return Err(self.failed(&format!("read from the {} engine: {e}", self.name)))
                }
            };
            if n == 0 {
                self.reap();
                return Err(self.failed(&format!("{} engine stopped unexpectedly", self.name)));
            }
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(response) => return Ok(response),
                Err(_) => log::warn!("{} engine: skipped a line that is not protocol", self.name),
            }
        }
    }

    /// Make sure the engine has exited and all of its stderr has been read,
    /// so the last line explains why it stopped.
    fn reap(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }

    /// `what`, plus the engine's last stderr line if there is one. Logs keep
    /// only the part before the first `": "`, so stderr (which can quote the
    /// text) never reaches them.
    fn failed(&self, what: &str) -> String {
        let tail = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        match tail.iter().rev().find(|l| !l.trim().is_empty()) {
            Some(line) => format!("{what}: {}", line.trim()),
            None => what.to_string(),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Closing stdin lets the engine exit; kill in case it is mid-request.
        self.stdin.take();
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.out_dir);
    }
}
