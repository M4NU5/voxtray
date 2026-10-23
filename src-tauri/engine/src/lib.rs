//! Voxtray's speech engine protocol, and the host-side client that drives an
//! engine process with it. The engine itself is the `voxtray-engine` binary.

mod client;
pub mod protocol;

pub use client::Client;

/// The Kokoro lexicon file for `voice`, which the engine loads with the
/// model (see `--voice`). British voices (`bf_`, `bm_`) read with the British
/// lexicon. Words not in it fall back to espeak-ng's US English: the bundled
/// espeak data has no en-gb voice, so setting a language would fail.
pub fn kokoro_lexicon(voice: &str) -> &'static str {
    if voice.starts_with('b') {
        "lexicon-gb-en.txt"
    } else {
        "lexicon-us-en.txt"
    }
}

/// The value after `--name` in `args`, if present.
pub fn arg(args: &[String], name: &str) -> Option<String> {
    let at = args.iter().position(|a| a == name)?;
    args.get(at + 1).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn british_voices_use_the_british_lexicon() {
        assert_eq!(kokoro_lexicon("bf_emma"), "lexicon-gb-en.txt");
        assert_eq!(kokoro_lexicon("af_heart"), "lexicon-us-en.txt");
    }
}
