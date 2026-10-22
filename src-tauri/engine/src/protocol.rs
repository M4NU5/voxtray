//! The engine protocol: one JSON object per line.
//!
//! The engine starts, loads its model and sends `ready`. The host then sends
//! `synthesize` requests one at a time. Each is answered by `result` (a WAV
//! file the host reads and deletes) or `error`, carrying the request's id.
//! `quit`, or the end of input, stops the engine. Stdout carries only these
//! lines; logs go to stderr.
//!
//! ```text
//! engine: {"type":"ready","protocol":1,"engine":"kokoro","version":"0.2.0","sample_rate":24000}
//! host:   {"type":"synthesize","id":1,"text":"Hello.","voice":"af_heart","speed":1.0}
//! engine: {"type":"result","id":1,"path":"/tmp/voxtray-kokoro-…/1.wav"}
//! host:   {"type":"quit"}
//! ```
//!
//! Nothing here depends on stdio, so the same messages could later travel
//! over a socket.

use std::fs::File;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped on any incompatible change. The host refuses other versions.
pub const PROTOCOL: u32 = 1;

/// Host to engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Speak `text`. Answered by `result` or `error` with the same id.
    Synthesize {
        id: u64,
        text: String,
        voice: String,
        speed: f32,
    },
    /// Exit now. Closing the input does the same.
    Quit,
}

/// What the engine reports once its model has loaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ready {
    pub protocol: u32,
    pub engine: String,
    pub version: String,
    pub sample_rate: u32,
}

/// Engine to host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Sent once, first, when the engine can take requests.
    Ready(Ready),
    /// The audio for request `id`: a mono 16-bit WAV file.
    Result { id: u64, path: PathBuf },
    /// Request `id` failed. Without an id, the engine could not start or
    /// could not read a request. Never contains the text being read.
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<u64>,
        message: String,
    },
}

/// A speech model behind the protocol.
pub trait Backend {
    /// Short engine name for `ready`, e.g. `kokoro`.
    fn name(&self) -> &str;
    fn sample_rate(&self) -> u32;
    /// Mono samples in [-1, 1] at [`Backend::sample_rate`].
    fn synthesize(&mut self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>, String>;
}

/// Write one message as a line and flush it.
pub fn send(out: &mut impl Write, response: &Response) -> io::Result<()> {
    serde_json::to_writer(&mut *out, response)?;
    out.write_all(b"\n")?;
    out.flush()
}

/// Send `ready`, then answer requests from `input` until `quit` or the end
/// of input. Audio files are written to `out_dir` as `<id>.wav`.
pub fn serve(
    input: impl BufRead,
    mut output: impl Write,
    backend: &mut impl Backend,
    out_dir: &Path,
) -> io::Result<()> {
    let rate = backend.sample_rate();
    let ready = Ready {
        protocol: PROTOCOL,
        engine: backend.name().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        sample_rate: rate,
    };
    send(&mut output, &Response::Ready(ready))?;

    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(Request::Quit) => break,
            Ok(Request::Synthesize {
                id,
                text,
                voice,
                speed,
            }) => {
                let path = out_dir.join(format!("{id}.wav"));
                let written = backend
                    .synthesize(&text, &voice, speed)
                    .and_then(|samples| {
                        write_wav(&path, &samples, rate).map_err(|e| format!("write audio: {e}"))
                    });
                match written {
                    Ok(()) => Response::Result { id, path },
                    Err(message) => Response::Error {
                        id: Some(id),
                        message,
                    },
                }
            }
            Err(e) => Response::Error {
                id: None,
                message: format!("bad request: {e}"),
            },
        };
        send(&mut output, &reply)?;
    }
    Ok(())
}

/// Save mono samples in [-1, 1] as a 16-bit PCM WAV file.
pub fn write_wav(path: &Path, samples: &[f32], rate: u32) -> io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    let data_len = (samples.len() * 2) as u32;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_len).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    // Chunk size, PCM, mono, rate, byte rate, block align, bits per sample.
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&rate.to_le_bytes())?;
    w.write_all(&(rate * 2).to_le_bytes())?;
    w.write_all(&2u16.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        w.write_all(&v.to_le_bytes())?;
    }
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 ms of a constant level per character at 1 kHz.
    struct Steady;

    impl Backend for Steady {
        fn name(&self) -> &str {
            "steady"
        }
        fn sample_rate(&self) -> u32 {
            1000
        }
        fn synthesize(&mut self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>, String> {
            if voice != "ok" {
                return Err(format!("unknown voice {voice}"));
            }
            Ok(vec![0.5 * speed; text.chars().count()])
        }
    }

    fn out_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("voxtray-protocol-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Run `serve` over `input` and parse every line it wrote.
    fn run(input: &str, dir: &Path) -> Vec<Response> {
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, &mut Steady, dir).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn messages_have_the_documented_shape() {
        let req = Request::Synthesize {
            id: 1,
            text: "Hello.".into(),
            voice: "af_heart".into(),
            speed: 1.2,
        };
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"type":"synthesize","id":1,"text":"Hello.","voice":"af_heart","speed":1.2}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::Quit).unwrap(),
            r#"{"type":"quit"}"#
        );
        let err = Response::Error {
            id: None,
            message: "no model".into(),
        };
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"type":"error","message":"no model"}"#
        );
        let ready: Response = serde_json::from_str(
            r#"{"type":"ready","protocol":1,"engine":"kokoro","version":"0.2.0","sample_rate":24000}"#,
        )
        .unwrap();
        assert!(matches!(ready, Response::Ready(r) if r.sample_rate == 24000));
    }

    #[test]
    fn ready_comes_first_then_one_answer_per_request() {
        let dir = out_dir("answers");
        let input = concat!(
            r#"{"type":"synthesize","id":7,"text":"abcd","voice":"ok","speed":1.0}"#,
            "\n\n",
            r#"{"type":"synthesize","id":8,"text":"ab","voice":"nope","speed":1.0}"#,
            "\n",
        );
        let replies = run(input, &dir);
        assert_eq!(replies.len(), 3);
        let Response::Ready(ready) = &replies[0] else {
            panic!("expected ready, got {:?}", replies[0]);
        };
        assert_eq!(
            (ready.protocol, ready.engine.as_str()),
            (PROTOCOL, "steady")
        );
        assert_eq!(
            replies[1],
            Response::Result {
                id: 7,
                path: dir.join("7.wav")
            }
        );
        assert_eq!(
            replies[2],
            Response::Error {
                id: Some(8),
                message: "unknown voice nope".into()
            }
        );
        // 44-byte header, then 4 samples of 16-bit audio.
        let wav = std::fs::read(dir.join("7.wav")).unwrap();
        assert_eq!(wav.len(), 44 + 8);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(i16::from_le_bytes([wav[44], wav[45]]), i16::MAX / 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_bad_line_gets_an_error_and_the_engine_keeps_going() {
        let dir = out_dir("bad-line");
        let input = concat!(
            "not json\n",
            r#"{"type":"cancel"}"#,
            "\n",
            r#"{"type":"synthesize","id":1,"text":"a","voice":"ok","speed":1.0}"#,
            "\n",
        );
        let replies = run(input, &dir);
        assert!(
            matches!(&replies[1], Response::Error { id: None, message } if message.starts_with("bad request"))
        );
        assert!(matches!(&replies[2], Response::Error { id: None, .. }));
        assert!(matches!(&replies[3], Response::Result { id: 1, .. }));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn quit_stops_before_later_requests() {
        let dir = out_dir("quit");
        let input = concat!(
            r#"{"type":"quit"}"#,
            "\n",
            r#"{"type":"synthesize","id":1,"text":"a","voice":"ok","speed":1.0}"#,
            "\n",
        );
        assert_eq!(run(input, &dir).len(), 1, "only ready");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wav_samples_are_clamped() {
        let dir = out_dir("clamp");
        let path = dir.join("c.wav");
        write_wav(&path, &[2.0, -2.0], 24_000).unwrap();
        let wav = std::fs::read(&path).unwrap();
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 24_000);
        assert_eq!(i16::from_le_bytes([wav[44], wav[45]]), i16::MAX);
        assert_eq!(i16::from_le_bytes([wav[46], wav[47]]), -i16::MAX);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
