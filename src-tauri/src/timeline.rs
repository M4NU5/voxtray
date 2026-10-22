//! The audio timeline for one reading.
//!
//! Each synthesised chunk is decoded to PCM once and appended in order. One
//! cursor walks across all chunks, so skipping ±5 s works across chunk
//! boundaries (rodio's own `try_seek`/`get_pos` only cover a single source).
//! [`TimelineSource`] is the rodio source that plays from the cursor. Pause
//! and underruns play silence without moving the cursor.

use std::io::Cursor;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use rodio::{Decoder, Source};

/// Samples copied per lock of the timeline. About 20 ms of mono speech, which
/// bounds how long a pause or skip takes to be heard.
const BLOCK: usize = 512;
/// Rate used for silence before any audio exists.
const FALLBACK_RATE: u32 = 24_000;

/// Decoded 16-bit PCM for one chunk. Samples are interleaved by channel.
#[derive(Clone, Debug, PartialEq)]
pub struct Pcm {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Pcm {
    /// Raw signed 16-bit little-endian PCM, as returned by the cloud providers.
    pub fn from_s16le(bytes: &[u8], sample_rate: u32, channels: u16) -> Pcm {
        let samples = bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        Pcm {
            samples,
            sample_rate,
            channels,
        }
    }

    /// Decode a complete encoded clip (WAV from Piper).
    pub fn decode(bytes: Vec<u8>) -> Result<Pcm, String> {
        let decoder = Decoder::new(Cursor::new(bytes)).map_err(|e| format!("decode audio: {e}"))?;
        let sample_rate = decoder.sample_rate();
        let channels = decoder.channels();
        Ok(Pcm {
            samples: decoder.collect(),
            sample_rate,
            channels,
        })
    }

    fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    pub fn duration(&self) -> Duration {
        frames_to_duration(self.frames() as u64, self.sample_rate)
    }

    /// Sample offset of the frame at `d` from the start, clamped to the end.
    fn offset_at(&self, d: Duration) -> usize {
        let frames = (d.as_nanos() * u128::from(self.sample_rate) / 1_000_000_000) as usize;
        frames.min(self.frames()) * usize::from(self.channels.max(1))
    }
}

fn frames_to_duration(frames: u64, rate: u32) -> Duration {
    if rate == 0 {
        return Duration::ZERO;
    }
    Duration::from_nanos((u128::from(frames) * 1_000_000_000 / u128::from(rate)) as u64)
}

/// What the listener is hearing right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Waiting for audio: before the first chunk, or synthesis fell behind.
    Loading,
    Playing,
    Paused,
    /// Played to the end, or stopped.
    Finished,
}

/// All audio for one reading, plus the play cursor.
#[derive(Debug, Default)]
pub struct Timeline {
    chunks: Vec<Pcm>,
    /// Index of the chunk under the cursor.
    chunk: usize,
    /// Sample offset into `chunks[chunk]`.
    offset: usize,
    /// No more chunks will be appended.
    complete: bool,
    paused: bool,
    finished: bool,
}

impl Timeline {
    pub fn push(&mut self, pcm: Pcm) {
        self.chunks.push(pcm);
    }

    /// Mark that every chunk has been appended.
    pub fn complete(&mut self) {
        self.complete = true;
    }

    /// End the reading now. The source stops at its next block.
    pub fn stop(&mut self) {
        self.finished = true;
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn state(&self) -> State {
        if self.finished {
            State::Finished
        } else if self.paused {
            State::Paused
        } else if self.at_end() && !self.complete {
            State::Loading
        } else {
            State::Playing
        }
    }

    /// Chunks appended but not yet fully played, including the current one.
    pub fn chunks_ahead(&self) -> usize {
        self.chunks.len().saturating_sub(self.chunk)
    }

    pub fn position(&self) -> Duration {
        let before: Duration = self.chunks[..self.chunk.min(self.chunks.len())]
            .iter()
            .map(Pcm::duration)
            .sum();
        let within = self.chunks.get(self.chunk).map_or(Duration::ZERO, |c| {
            frames_to_duration(
                (self.offset / usize::from(c.channels.max(1))) as u64,
                c.sample_rate,
            )
        });
        before + within
    }

    #[cfg(test)]
    pub fn chunk_durations(&self) -> Vec<Duration> {
        self.chunks.iter().map(Pcm::duration).collect()
    }

    /// Total audio appended so far.
    pub fn buffered(&self) -> Duration {
        self.chunks.iter().map(Pcm::duration).sum()
    }

    /// Move the cursor to `pos`, clamped to the audio synthesised so far.
    /// Seeking to the end of a complete reading finishes it.
    pub fn seek_to(&mut self, pos: Duration) {
        let mut remaining = pos;
        for (i, c) in self.chunks.iter().enumerate() {
            let d = c.duration();
            if remaining < d {
                self.chunk = i;
                self.offset = c.offset_at(remaining);
                return;
            }
            remaining -= d;
        }
        // At or past the end of the buffered audio.
        self.chunk = self.chunks.len().saturating_sub(1);
        self.offset = self.chunks.last().map_or(0, |c| c.samples.len());
    }

    /// Skip forward (positive) or back (negative) by `delta`.
    pub fn skip(&mut self, delta: Duration, forward: bool) {
        let pos = self.position();
        let target = if forward {
            pos + delta
        } else {
            pos.saturating_sub(delta)
        };
        self.seek_to(target);
    }

    fn at_end(&self) -> bool {
        match self.chunks.get(self.chunk) {
            None => true,
            Some(c) => self.offset >= c.samples.len() && self.chunk + 1 >= self.chunks.len(),
        }
    }

    /// Copy up to `max` samples from one chunk into `out`.
    fn read_block(&mut self, out: &mut Vec<i16>, max: usize) -> Block {
        out.clear();
        if self.finished {
            return Block::End;
        }
        // Step over finished (or empty) chunks.
        while self.chunk + 1 < self.chunks.len()
            && self.offset >= self.chunks[self.chunk].samples.len()
        {
            self.chunk += 1;
            self.offset = 0;
        }
        let rate = self
            .chunks
            .get(self.chunk)
            .map_or(FALLBACK_RATE, |c| c.sample_rate);
        let channels = self.chunks.get(self.chunk).map_or(1, |c| c.channels);
        if self.paused {
            return Block::Silence { rate, channels };
        }
        if self.at_end() {
            if self.complete {
                self.finished = true;
                return Block::End;
            }
            return Block::Silence { rate, channels };
        }
        let c = &self.chunks[self.chunk];
        let end = (self.offset + max).min(c.samples.len());
        out.extend_from_slice(&c.samples[self.offset..end]);
        self.offset = end;
        Block::Audio { rate, channels }
    }
}

enum Block {
    Audio { rate: u32, channels: u16 },
    Silence { rate: u32, channels: u16 },
    End,
}

/// A timeline shared between the synthesis worker, the hotkeys and the audio
/// thread. `changed` is signalled whenever the cursor crosses into another
/// chunk or the reading finishes.
#[derive(Debug, Default)]
pub struct Shared {
    pub timeline: Mutex<Timeline>,
    pub changed: Condvar,
}

impl Shared {
    pub fn new() -> Arc<Shared> {
        Arc::new(Shared::default())
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut Timeline) -> T) -> T {
        let mut t = self.timeline.lock().unwrap_or_else(|e| e.into_inner());
        let out = f(&mut t);
        self.changed.notify_all();
        out
    }
}

/// Plays a [`Shared`] timeline from its cursor. Ends when the timeline does.
pub struct TimelineSource {
    shared: Arc<Shared>,
    buf: Vec<i16>,
    pos: usize,
    rate: u32,
    channels: u16,
    ended: bool,
}

impl TimelineSource {
    pub fn new(shared: Arc<Shared>) -> Self {
        let mut s = TimelineSource {
            shared,
            buf: Vec::with_capacity(BLOCK),
            pos: 0,
            rate: FALLBACK_RATE,
            channels: 1,
            ended: false,
        };
        s.refill();
        s
    }

    fn refill(&mut self) {
        let mut t = self
            .shared
            .timeline
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let chunk_before = t.chunk;
        let block = t.read_block(&mut self.buf, BLOCK);
        let crossed = t.chunk != chunk_before;
        drop(t);
        self.pos = 0;
        match block {
            Block::Audio { rate, channels } => {
                self.rate = rate;
                self.channels = channels;
            }
            Block::Silence { rate, channels } => {
                self.rate = rate;
                self.channels = channels;
                // ~10 ms of silence, a whole number of frames.
                let n = (rate as usize / 100).max(1) * usize::from(channels.max(1));
                self.buf.resize(n, 0);
            }
            Block::End => {
                self.ended = true;
                self.shared.changed.notify_all();
            }
        }
        if crossed {
            self.shared.changed.notify_all();
        }
    }
}

impl Iterator for TimelineSource {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        if self.ended {
            return None;
        }
        let sample = self.buf[self.pos];
        self.pos += 1;
        if self.pos >= self.buf.len() {
            self.refill();
        }
        Some(sample)
    }
}

impl Source for TimelineSource {
    fn current_frame_len(&self) -> Option<usize> {
        // Each block comes from one chunk, so format changes only at block edges.
        Some(if self.ended {
            0
        } else {
            self.buf.len() - self.pos
        })
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mono chunk at 1 kHz whose samples count up from `start`, so one
    /// sample is one millisecond and every sample says where it came from.
    fn ramp(start: i16, ms: usize) -> Pcm {
        Pcm {
            samples: (0..ms).map(|i| start + i as i16).collect(),
            sample_rate: 1000,
            channels: 1,
        }
    }

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(s)
    }

    fn pull(src: &mut TimelineSource, n: usize) -> Vec<i16> {
        src.take(n).collect()
    }

    fn timeline(chunks: &[Pcm], complete: bool) -> Arc<Shared> {
        let shared = Shared::new();
        shared.with(|t| {
            for c in chunks {
                t.push(c.clone());
            }
            if complete {
                t.complete();
            }
        });
        shared
    }

    #[test]
    fn plays_chunks_back_to_back_then_ends() {
        let shared = timeline(&[ramp(0, 3), ramp(100, 2)], true);
        let all: Vec<i16> = TimelineSource::new(shared.clone()).collect();
        assert_eq!(all, [0, 1, 2, 100, 101]);
        assert_eq!(shared.with(|t| t.state()), State::Finished);
    }

    #[test]
    fn skip_back_crosses_into_the_previous_chunk() {
        // Two 10 s chunks. 1 s into the second, skip back 5 s: 6 s into the first.
        let shared = timeline(&[ramp(0, 10_000), ramp(20_000, 10_000)], true);
        shared.with(|t| t.seek_to(secs(11.0)));
        shared.with(|t| t.skip(secs(5.0), false));
        assert_eq!(shared.with(|t| t.position()), secs(6.0));
        let mut src = TimelineSource::new(shared);
        assert_eq!(pull(&mut src, 2), [6_000, 6_001]);
    }

    #[test]
    fn skip_forward_crosses_into_the_next_chunk() {
        let shared = timeline(&[ramp(0, 10_000), ramp(20_000, 10_000)], true);
        shared.with(|t| t.seek_to(secs(8.0)));
        shared.with(|t| t.skip(secs(5.0), true));
        assert_eq!(shared.with(|t| t.position()), secs(13.0));
        let mut src = TimelineSource::new(shared);
        assert_eq!(pull(&mut src, 1), [23_000]);
    }

    #[test]
    fn skip_back_clamps_to_the_start() {
        let shared = timeline(&[ramp(0, 10_000)], true);
        shared.with(|t| t.seek_to(secs(2.0)));
        shared.with(|t| t.skip(secs(5.0), false));
        assert_eq!(shared.with(|t| t.position()), Duration::ZERO);
    }

    #[test]
    fn skip_forward_past_synthesised_audio_waits_at_the_edge() {
        // Synthesis is behind: the cursor parks at the end and plays silence
        // until the next chunk arrives, then carries on from there.
        let shared = timeline(&[ramp(0, 3_000)], false);
        shared.with(|t| t.seek_to(secs(1.0)));
        shared.with(|t| t.skip(secs(5.0), true));
        assert_eq!(shared.with(|t| t.position()), secs(3.0));
        assert_eq!(shared.with(|t| t.state()), State::Loading);
        let mut src = TimelineSource::new(shared.clone());
        assert!(pull(&mut src, 50).iter().all(|&s| s == 0));
        shared.with(|t| t.push(ramp(500, 10)));
        let after: Vec<i16> = src.by_ref().skip_while(|&s| s == 0).take(3).collect();
        assert_eq!(after, [500, 501, 502]);
    }

    #[test]
    fn skip_forward_past_the_end_of_a_complete_reading_finishes_it() {
        let shared = timeline(&[ramp(1, 3_000)], true);
        shared.with(|t| t.skip(secs(5.0), true));
        let rest: Vec<i16> = TimelineSource::new(shared.clone()).collect();
        assert!(rest.is_empty());
        assert_eq!(shared.with(|t| t.state()), State::Finished);
    }

    #[test]
    fn pause_plays_silence_and_holds_the_cursor() {
        let shared = timeline(&[ramp(1, 5_000)], true);
        let mut src = TimelineSource::new(shared.clone());
        assert_eq!(pull(&mut src, BLOCK - 1)[0], 1);
        shared.with(|t| t.set_paused(true));
        assert_eq!(shared.with(|t| t.state()), State::Paused);
        // The rest of the block already copied still plays, then silence.
        assert_eq!(src.next(), Some(BLOCK as i16));
        let held = shared.with(|t| t.position());
        assert!(pull(&mut src, 2_000).iter().all(|&s| s == 0));
        assert_eq!(shared.with(|t| t.position()), held);
        shared.with(|t| t.set_paused(false));
        let resumed: Vec<i16> = src.by_ref().skip_while(|&s| s == 0).take(1).collect();
        assert_eq!(resumed, [1 + BLOCK as i16]);
    }

    #[test]
    fn stop_ends_the_source() {
        let shared = timeline(&[ramp(1, 5_000)], false);
        let mut src = TimelineSource::new(shared.clone());
        shared.with(|t| t.stop());
        // Whatever was already buffered drains, then the source ends.
        assert!(src.by_ref().count() <= BLOCK);
        assert_eq!(shared.with(|t| t.state()), State::Finished);
    }

    #[test]
    fn chunks_ahead_counts_from_the_cursor() {
        let shared = timeline(&[ramp(0, 10), ramp(0, 10), ramp(0, 10)], false);
        assert_eq!(shared.with(|t| t.chunks_ahead()), 3);
        shared.with(|t| t.seek_to(Duration::from_millis(15)));
        assert_eq!(shared.with(|t| t.chunks_ahead()), 2);
    }

    #[test]
    fn real_rates_seek_sample_exactly() {
        // Piper voices are 22.05 kHz: 5 s is exactly 110_250 frames.
        let pcm = Pcm {
            samples: (0..22_050 * 12).map(|i| (i % 30_000) as i16).collect(),
            sample_rate: 22_050,
            channels: 1,
        };
        let shared = timeline(&[pcm], true);
        shared.with(|t| t.skip(secs(5.0), true));
        let mut src = TimelineSource::new(shared);
        assert_eq!(pull(&mut src, 1), [(110_250 % 30_000) as i16]);
    }

    #[test]
    fn s16le_bytes_decode_in_order() {
        let pcm = Pcm::from_s16le(&[0x01, 0x00, 0xff, 0xff, 0x00, 0x80], 24_000, 1);
        assert_eq!(pcm.samples, [1, -1, i16::MIN]);
        assert_eq!(pcm.duration(), Duration::from_nanos(125_000));
    }

    #[test]
    fn plays_through_a_real_rodio_sink_without_a_device() {
        // Two chunks at different rates. The sink's queue must pass each
        // block's format through so the device mixer can resample it.
        let a = Pcm {
            samples: vec![1000; 2_400],
            sample_rate: 24_000,
            channels: 1,
        };
        let b = Pcm {
            samples: vec![-1000; 2_205],
            sample_rate: 22_050,
            channels: 1,
        };
        let shared = timeline(&[a, b], true);
        let (sink, mut output) = rodio::Sink::new_idle();
        sink.append(TimelineSource::new(shared.clone()));
        let first: Vec<f32> = output.by_ref().take(2_400).collect();
        assert!(first.iter().all(|&s| s > 0.0), "{:?}", &first[..8]);
        assert_eq!(output.sample_rate(), 22_050);
        let second: Vec<f32> = output.by_ref().take(2_205).collect();
        assert!(second.iter().all(|&s| s < 0.0), "{:?}", &second[..8]);
        output.next();
        assert_eq!(shared.with(|t| t.state()), State::Finished);
    }
}
