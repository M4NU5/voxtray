//! Kokoro v1.0 through sherpa-onnx's C API.

use std::path::Path;

use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig};
use voxtray_engine::protocol::Backend;

/// Voice names in speaker-id order for `kokoro-multi-lang-v1_0`. From
/// <https://k2-fsa.github.io/sherpa/onnx/tts/pretrained_models/kokoro.html>
/// and the model's README (em_santa was added as 53).
const VOICES: [&str; 54] = [
    "af_alloy",
    "af_aoede",
    "af_bella",
    "af_heart",
    "af_jessica",
    "af_kore",
    "af_nicole",
    "af_nova",
    "af_river",
    "af_sarah",
    "af_sky",
    "am_adam",
    "am_echo",
    "am_eric",
    "am_fenrir",
    "am_liam",
    "am_michael",
    "am_onyx",
    "am_puck",
    "am_santa",
    "bf_alice",
    "bf_emma",
    "bf_isabella",
    "bf_lily",
    "bm_daniel",
    "bm_fable",
    "bm_george",
    "bm_lewis",
    "ef_dora",
    "em_alex",
    "ff_siwis",
    "hf_alpha",
    "hf_beta",
    "hm_omega",
    "hm_psi",
    "if_sara",
    "im_nicola",
    "jf_alpha",
    "jf_gongitsune",
    "jf_nezumi",
    "jf_tebukuro",
    "jm_kumo",
    "pf_dora",
    "pm_alex",
    "pm_santa",
    "zf_xiaobei",
    "zf_xiaoni",
    "zf_xiaoxiao",
    "zf_xiaoyi",
    "zm_yunjian",
    "zm_yunxi",
    "zm_yunxia",
    "zm_yunyang",
    "em_santa",
];

/// The speaker id for a voice name.
pub fn sid(voice: &str) -> Option<i32> {
    VOICES.iter().position(|v| *v == voice).map(|i| i as i32)
}

/// British voices (`bf_`, `bm_`) read with the British lexicon. Words not in
/// it fall back to espeak-ng's US English: the bundled espeak data has no
/// en-gb voice, so setting a language would fail.
fn lexicon(voice: &str) -> &'static str {
    if voice.starts_with('b') {
        "lexicon-gb-en.txt"
    } else {
        "lexicon-us-en.txt"
    }
}

pub struct Kokoro {
    tts: OfflineTts,
    rate: u32,
}

impl Kokoro {
    /// Load the model in `dir`, with the lexicon for `voice`.
    pub fn load(dir: &Path, voice: &str, threads: i32) -> Result<Kokoro, String> {
        let file = |name: &str| Some(dir.join(name).to_string_lossy().into_owned());
        let mut config = OfflineTtsConfig::default();
        config.model.kokoro = OfflineTtsKokoroModelConfig {
            model: file("model.onnx"),
            voices: file("voices.bin"),
            tokens: file("tokens.txt"),
            data_dir: file("espeak-ng-data"),
            lexicon: file(lexicon(voice)),
            ..Default::default()
        };
        config.model.num_threads = threads;
        let tts = OfflineTts::create(&config).ok_or("could not load the Kokoro model")?;
        let rate = u32::try_from(tts.sample_rate()).map_err(|_| "bad sample rate")?;
        Ok(Kokoro { tts, rate })
    }
}

impl Backend for Kokoro {
    fn name(&self) -> &str {
        "kokoro"
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn synthesize(&mut self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>, String> {
        let sid = sid(voice).ok_or_else(|| format!("unknown Kokoro voice {voice}"))?;
        let config = GenerationConfig {
            sid,
            speed,
            ..Default::default()
        };
        // The C API takes a C string, which can't hold NUL.
        let text = text.replace('\0', " ");
        match self
            .tts
            .generate_with_config(&text, &config, None::<fn(&[f32], f32) -> bool>)
        {
            Some(audio) => Ok(audio.samples().to_vec()),
            // Nothing Kokoro can pronounce, such as text in another script.
            // A moment of silence keeps the rest of the reading going.
            None => {
                eprintln!("a chunk had nothing to say; sending silence");
                Ok(vec![0.0; self.rate as usize / 10])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_map_to_documented_speaker_ids() {
        assert_eq!(sid("af_alloy"), Some(0));
        assert_eq!(sid("af_heart"), Some(3));
        assert_eq!(sid("am_michael"), Some(16));
        assert_eq!(sid("bf_emma"), Some(21));
        assert_eq!(sid("bm_lewis"), Some(27));
        assert_eq!(sid("zm_yunyang"), Some(52));
        assert_eq!(sid("em_santa"), Some(53));
        assert_eq!(sid("nobody"), None);
    }

    #[test]
    fn british_voices_use_the_british_lexicon() {
        assert_eq!(lexicon("bf_emma"), "lexicon-gb-en.txt");
        assert_eq!(lexicon("af_heart"), "lexicon-us-en.txt");
    }
}
