//! Playback of chunked readings (a PCM timeline via rodio) and the local
//! system voice (via a subprocess), with a single `stop()` that covers both.

use std::process::Child;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use rodio::{OutputStream, Sink};

use crate::timeline::{Shared, TimelineSource};

enum AudioCmd {
    Play(Arc<Shared>),
    Stop,
}

/// Owns the audio output thread and any running local-voice process.
pub struct Player {
    audio_tx: Sender<AudioCmd>,
    local_child: Arc<Mutex<Option<Child>>>,
}

impl Player {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<AudioCmd>();

        // Dedicated audio thread: the rodio OutputStream is not Send, so it must
        // live entirely on one thread. Commands arrive over the channel.
        std::thread::spawn(move || {
            let stream = OutputStream::try_default();
            let (_stream, handle) = match stream {
                Ok(s) => s,
                Err(e) => {
                    log::error!("audio output unavailable: {e}");
                    return;
                }
            };
            let mut current: Option<Sink> = None;
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    AudioCmd::Play(shared) => {
                        if let Some(s) = current.take() {
                            s.stop();
                        }
                        match Sink::try_new(&handle) {
                            Ok(sink) => {
                                sink.append(TimelineSource::new(shared));
                                current = Some(sink);
                            }
                            Err(e) => {
                                log::error!("create sink: {e}");
                                shared.with(|t| t.stop());
                            }
                        }
                    }
                    AudioCmd::Stop => {
                        if let Some(s) = current.take() {
                            s.stop();
                        }
                    }
                }
            }
        });

        Player {
            audio_tx: tx,
            local_child: Arc::new(Mutex::new(None)),
        }
    }

    /// Play a reading's timeline from its cursor. Audio appended to the
    /// timeline later is picked up as playback reaches it.
    pub fn play_timeline(&self, shared: Arc<Shared>) -> Result<(), String> {
        self.stop_local();
        self.audio_tx
            .send(AudioCmd::Play(shared))
            .map_err(|_| "audio output unavailable".to_string())
    }

    /// Register the child process spawned for local system-voice playback so it
    /// can be stopped later. Also stops any cloud audio.
    pub fn set_local_child(&self, child: Child) {
        let _ = self.audio_tx.send(AudioCmd::Stop);
        if let Ok(mut guard) = self.local_child.lock() {
            if let Some(mut old) = guard.take() {
                let _ = old.kill();
            }
            *guard = Some(child);
        }
    }

    fn stop_local(&self) {
        if let Ok(mut guard) = self.local_child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
            }
        }
    }

    /// Stop everything currently playing.
    pub fn stop(&self) {
        let _ = self.audio_tx.send(AudioCmd::Stop);
        self.stop_local();
    }
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}
