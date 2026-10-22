//! One reading: synthesises chunks a little ahead of playback into a shared
//! timeline, and exposes pause, skip and stop.
//!
//! States, as seen by the hotkeys: no reading (idle) -> Loading -> Playing <->
//! Paused -> Finished (idle again). Stop moves any state to Finished and
//! cancels synthesis that hasn't happened yet.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::timeline::{Pcm, Shared, State};

/// Chunks synthesised ahead of the one playing.
pub const LOOKAHEAD: usize = 2;
/// How far the skip hotkeys jump.
pub const SKIP: Duration = Duration::from_secs(5);

/// How a reading ended.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Finished,
    Stopped,
    /// Synthesis failed. Audio made before the failure was still played.
    Failed(String),
}

type Interrupt = Box<dyn FnOnce() + Send>;

pub struct Reading {
    pub id: u64,
    pub shared: Arc<Shared>,
    cancelled: AtomicBool,
    interrupt: Mutex<Option<Interrupt>>,
}

impl Reading {
    /// Start synthesising `chunks` in order on a worker thread. `synth` turns
    /// one chunk into audio; `interrupt` aborts a call to it in progress.
    /// `done` runs on the worker thread once playback has ended.
    pub fn start<S, D>(
        chunks: Vec<String>,
        synth: S,
        interrupt: Option<Interrupt>,
        done: D,
    ) -> Arc<Reading>
    where
        S: FnMut(&str) -> Result<Pcm, String> + Send + 'static,
        D: FnOnce(&Reading, Outcome) + Send + 'static,
    {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let reading = Arc::new(Reading {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            shared: Shared::new(),
            cancelled: AtomicBool::new(false),
            interrupt: Mutex::new(interrupt),
        });
        let r = reading.clone();
        std::thread::spawn(move || {
            let outcome = r.run(chunks, synth);
            log::info!("reading {}: {outcome:?}", r.id);
            done(&r, outcome);
        });
        reading
    }

    fn run<S>(&self, chunks: Vec<String>, mut synth: S) -> Outcome
    where
        S: FnMut(&str) -> Result<Pcm, String>,
    {
        let started = Instant::now();
        log::info!("reading {}: {} chunks", self.id, chunks.len());
        for (i, text) in chunks.iter().enumerate() {
            if !self.wait_for_room() {
                return Outcome::Stopped;
            }
            let t = Instant::now();
            let result = synth(text);
            if self.is_cancelled() {
                return Outcome::Stopped;
            }
            match result {
                Ok(pcm) => {
                    log::info!(
                        "reading {}: chunk {i} ({} chars) -> {:.2}s audio in {} ms{}",
                        self.id,
                        text.chars().count(),
                        pcm.duration().as_secs_f32(),
                        t.elapsed().as_millis(),
                        if i == 0 {
                            format!(", first audio after {} ms", started.elapsed().as_millis())
                        } else {
                            String::new()
                        }
                    );
                    self.shared.with(|t| t.push(pcm));
                }
                Err(e) => {
                    self.shared.with(|t| t.complete());
                    self.wait_until_finished();
                    return Outcome::Failed(e);
                }
            }
        }
        self.shared.with(|t| t.complete());
        if self.wait_until_finished() {
            Outcome::Finished
        } else {
            Outcome::Stopped
        }
    }

    /// Block until fewer than [`LOOKAHEAD`] chunks wait after the current one.
    /// False if the reading ended meanwhile.
    fn wait_for_room(&self) -> bool {
        let mut t = self
            .shared
            .timeline
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        while t.chunks_ahead() > LOOKAHEAD && t.state() != State::Finished && !self.is_cancelled() {
            t = self
                .shared
                .changed
                .wait(t)
                .unwrap_or_else(|e| e.into_inner());
        }
        t.state() != State::Finished && !self.is_cancelled()
    }

    /// Block until playback ends. True if it played out rather than stopped.
    fn wait_until_finished(&self) -> bool {
        let mut t = self
            .shared
            .timeline
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        while t.state() != State::Finished && !self.is_cancelled() {
            t = self
                .shared
                .changed
                .wait(t)
                .unwrap_or_else(|e| e.into_inner());
        }
        !self.is_cancelled()
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn state(&self) -> State {
        self.shared.with(|t| t.state())
    }

    /// Still playing, paused or loading.
    pub fn is_active(&self) -> bool {
        !self.is_cancelled() && self.state() != State::Finished
    }

    /// Pause if playing or loading, resume if paused. Returns the new state.
    pub fn toggle_pause(&self) -> State {
        self.shared.with(|t| {
            let pause = t.state() != State::Paused;
            t.set_paused(pause);
            t.state()
        })
    }

    /// Skip [`SKIP`] forward or back. Returns the new position and how much
    /// audio exists so far.
    pub fn skip(&self, forward: bool) -> (Duration, Duration) {
        self.shared.with(|t| {
            t.skip(SKIP, forward);
            (t.position(), t.buffered())
        })
    }

    /// End the reading and cancel any synthesis not yet done.
    pub fn stop(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.shared.with(|t| t.stop());
        let interrupt = self
            .interrupt
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(f) = interrupt {
            f();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::TimelineSource;
    use std::sync::mpsc;

    const WAIT: Duration = Duration::from_secs(5);

    /// 100 ms of non-zero audio at 1 kHz.
    fn clip() -> Pcm {
        Pcm {
            samples: vec![7; 100],
            sample_rate: 1000,
            channels: 1,
        }
    }

    fn chunks(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("chunk {i}")).collect()
    }

    /// Poll `f` until it holds or the wait runs out.
    fn eventually(f: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        f()
    }

    type Synth = Box<dyn FnMut(&str) -> Result<Pcm, String> + Send>;

    /// A fake engine that records which chunks it was asked for.
    fn recording_synth() -> (Arc<Mutex<Vec<String>>>, Synth) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let c = calls.clone();
        let synth = move |text: &str| {
            c.lock().unwrap().push(text.to_string());
            Ok(clip())
        };
        (calls, Box::new(synth))
    }

    fn done_channel() -> (
        mpsc::Receiver<Outcome>,
        impl FnOnce(&Reading, Outcome) + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        (rx, move |_: &Reading, o: Outcome| {
            let _ = tx.send(o);
        })
    }

    #[test]
    fn synthesises_only_lookahead_chunks_ahead_of_playback() {
        let (calls, synth) = recording_synth();
        let (_rx, done) = done_channel();
        let r = Reading::start(chunks(6), synth, None, done);
        // Nothing is playing: the current chunk plus LOOKAHEAD more, then wait.
        assert!(eventually(|| calls.lock().unwrap().len() == LOOKAHEAD + 1));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(calls.lock().unwrap().len(), LOOKAHEAD + 1);
        // Play past the first chunk: exactly one more is synthesised.
        let mut src = TimelineSource::new(r.shared.clone());
        src.by_ref().take(101).for_each(drop);
        assert!(eventually(|| calls.lock().unwrap().len() == LOOKAHEAD + 2));
        assert_eq!(calls.lock().unwrap()[..2], ["chunk 0", "chunk 1"]);
        r.stop();
    }

    #[test]
    fn plays_to_the_end_then_reports_finished() {
        let (_calls, synth) = recording_synth();
        let (rx, done) = done_channel();
        let r = Reading::start(chunks(5), synth, None, done);
        assert!(eventually(|| r.state() == State::Playing));
        let played: Vec<i16> = TimelineSource::new(r.shared.clone())
            .filter(|&s| s != 0)
            .collect();
        assert_eq!(played.len(), 500);
        assert_eq!(rx.recv_timeout(WAIT), Ok(Outcome::Finished));
        assert!(!r.is_active());
    }

    #[test]
    fn stop_cancels_pending_synthesis() {
        // The engine blocks until released, like a slow network request.
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let calls = Arc::new(Mutex::new(0));
        let c = calls.clone();
        let synth = move |_: &str| {
            *c.lock().unwrap() += 1;
            let _ = release_rx.recv();
            Ok(clip())
        };
        let interrupted = Arc::new(AtomicBool::new(false));
        let i = interrupted.clone();
        let interrupt: Interrupt = Box::new(move || i.store(true, Ordering::SeqCst));
        let (rx, done) = done_channel();

        let r = Reading::start(chunks(10), synth, Some(interrupt), done);
        assert!(eventually(|| *calls.lock().unwrap() == 1));
        r.stop();
        assert!(interrupted.load(Ordering::SeqCst));
        drop(release_tx);
        assert_eq!(rx.recv_timeout(WAIT), Ok(Outcome::Stopped));
        assert_eq!(*calls.lock().unwrap(), 1, "no chunk after stop");
        assert_eq!(
            r.shared.with(|t| t.buffered()),
            Duration::ZERO,
            "result discarded"
        );
        assert_eq!(r.state(), State::Finished);
    }

    #[test]
    fn a_failed_chunk_plays_what_exists_then_reports_the_error() {
        let mut n = 0;
        let synth = move |_: &str| {
            n += 1;
            if n == 3 {
                Err("network down".to_string())
            } else {
                Ok(clip())
            }
        };
        let (rx, done) = done_channel();
        let r = Reading::start(chunks(6), synth, None, done);
        let played = TimelineSource::new(r.shared.clone())
            .filter(|&s| s != 0)
            .count();
        assert_eq!(played, 200);
        assert_eq!(
            rx.recv_timeout(WAIT),
            Ok(Outcome::Failed("network down".into()))
        );
    }

    #[test]
    fn hotkey_states_follow_loading_playing_paused() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let synth = move |_: &str| {
            let _ = release_rx.recv();
            Ok(clip())
        };
        let (rx, done) = done_channel();
        let r = Reading::start(chunks(2), synth, None, done);
        assert_eq!(r.state(), State::Loading);
        release_tx.send(()).unwrap();
        assert!(eventually(|| r.state() == State::Playing));
        assert_eq!(r.toggle_pause(), State::Paused);
        assert!(r.is_active());
        assert_eq!(r.toggle_pause(), State::Playing);
        r.stop();
        assert!(!r.is_active());
        drop(release_tx);
        assert_eq!(rx.recv_timeout(WAIT), Ok(Outcome::Stopped));
    }

    #[test]
    fn skip_moves_by_five_seconds_across_chunks() {
        let long = || Pcm {
            samples: vec![1; 4_000],
            sample_rate: 1000,
            channels: 1,
        };
        let (rx, done) = done_channel();
        let r = Reading::start(chunks(3), move |_: &str| Ok(long()), None, done);
        assert!(eventually(
            || r.shared.with(|t| t.buffered()) == Duration::from_secs(12)
        ));
        assert_eq!(r.skip(true).0, Duration::from_secs(5));
        assert_eq!(r.skip(true).0, Duration::from_secs(10));
        assert_eq!(r.skip(false).0, Duration::from_secs(5));
        r.stop();
        assert_eq!(rx.recv_timeout(WAIT), Ok(Outcome::Stopped));
    }

    /// Renders a real passage through the real pipeline (chunker, warm Piper,
    /// reading worker, timeline source) and reports, per chunk, when its audio
    /// was ready versus when real-time playback would need it.
    ///
    /// `VOXTRAY_PIPER_EXE`, `VOXTRAY_PIPER_MODEL`, `VOXTRAY_RENDER_TEXT` (input
    /// file) and `VOXTRAY_RENDER_WAV` (output) must be set.
    #[test]
    #[ignore = "needs a Piper binary, a voice and a text file"]
    fn render_passage_with_piper() {
        use crate::tts::piper::Engine;
        use rodio::Source;
        use std::io::Write;

        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
        let text = std::fs::read_to_string(var("VOXTRAY_RENDER_TEXT")).unwrap();
        let chunks = crate::chunker::chunk(&text);
        let n = chunks.len();

        let started = Instant::now();
        let mut engine = Engine::spawn(
            var("VOXTRAY_PIPER_EXE").as_ref(),
            var("VOXTRAY_PIPER_MODEL").as_ref(),
            1.0,
        )
        .unwrap();
        let ready = Arc::new(Mutex::new(Vec::new()));
        let r2 = ready.clone();
        let synth = move |chunk: &str| {
            let pcm = engine.synthesize(chunk);
            r2.lock().unwrap().push(started.elapsed());
            pcm
        };
        let (rx, done) = done_channel();
        let r = Reading::start(chunks, synth, None, done);

        // Play like a device that never underruns: before the source refills,
        // wait until the timeline has audio past the cursor (or is complete).
        assert!(eventually(
            || r.shared.with(|t| t.buffered()) > Duration::ZERO
        ));
        let first_audio = started.elapsed();
        let mut src = TimelineSource::new(r.shared.clone());
        let rate = src.sample_rate();
        let mut samples: Vec<i16> = Vec::new();
        loop {
            if src.current_frame_len() == Some(1) {
                assert!(eventually(|| r.state() != State::Loading));
            }
            match src.next() {
                Some(s) => samples.push(s),
                None => break,
            }
        }
        assert_eq!(rx.recv_timeout(WAIT), Ok(Outcome::Finished));

        let mut wav =
            std::io::BufWriter::new(std::fs::File::create(var("VOXTRAY_RENDER_WAV")).unwrap());
        let data_len = (samples.len() * 2) as u32;
        wav.write_all(b"RIFF").unwrap();
        wav.write_all(&(36 + data_len).to_le_bytes()).unwrap();
        wav.write_all(b"WAVEfmt ").unwrap();
        for v in [16u32, 1 | (1 << 16), rate, rate * 2, 2 | (16 << 16)] {
            wav.write_all(&v.to_le_bytes()).unwrap();
        }
        wav.write_all(b"data").unwrap();
        wav.write_all(&data_len.to_le_bytes()).unwrap();
        for s in &samples {
            wav.write_all(&s.to_le_bytes()).unwrap();
        }
        wav.flush().unwrap();

        // Chunk i is needed once everything before it has played in real time.
        let ready = ready.lock().unwrap();
        let durations: Vec<Duration> = r.shared.with(|t| t.chunk_durations());
        let mut needed = first_audio;
        let mut worst = Duration::MAX;
        for (i, at) in ready.iter().enumerate() {
            if i > 0 {
                needed += durations[i - 1];
                worst = worst.min(needed.saturating_sub(*at));
                assert!(
                    *at <= needed,
                    "chunk {i} ready at {at:?}, needed at {needed:?}"
                );
            }
        }
        let audio = Duration::from_secs_f64(samples.len() as f64 / f64::from(rate));
        println!(
            "chunks={n} first_audio_ms={} synth_total_ms={} audio_s={:.1} min_headroom_ms={}",
            first_audio.as_millis(),
            ready.last().unwrap().as_millis(),
            audio.as_secs_f64(),
            worst.as_millis()
        );
    }
}
