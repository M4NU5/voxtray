//! OpenAI text-to-speech (`/v1/audio/speech`).

use crate::settings::{self, Provider, Settings};
use crate::timeline::Pcm;
use serde_json::{json, Value};
use std::time::Duration;

const ENDPOINT: &str = "https://api.openai.com/v1/audio/speech";
/// `response_format: "pcm"` is headerless 24 kHz signed 16-bit little-endian mono.
const PCM_RATE: u32 = 24_000;

/// One client per reading, so chunks reuse the same connection.
pub struct Client {
    http: reqwest::blocking::Client,
    key: String,
    model: String,
    voice: String,
    speed: f32,
}

impl Client {
    pub fn new(settings: &Settings) -> Result<Client, String> {
        let key = settings::get_api_key(Provider::Openai)
            .filter(|k| !k.trim().is_empty())
            .ok_or("No OpenAI API key set. Add one in settings.")?;
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Client {
            http,
            key,
            model: settings.openai_model.clone(),
            voice: settings.openai_voice.clone(),
            speed: settings.speed,
        })
    }

    pub fn synthesize(&self, text: &str) -> Result<Pcm, String> {
        let resp = self
            .http
            .post(ENDPOINT)
            .bearer_auth(&self.key)
            .json(&body(&self.model, &self.voice, self.speed, text))
            .send()
            .map_err(|e| format!("request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!(
                "OpenAI error {status}: {}",
                super::truncate(&text, 300)
            ));
        }

        let bytes = resp.bytes().map_err(|e| e.to_string())?;
        Ok(Pcm::from_s16le(&bytes, PCM_RATE, 1))
    }
}

fn body(model: &str, voice: &str, speed: f32, text: &str) -> Value {
    json!({
        "model": model,
        "voice": voice,
        "input": text,
        "response_format": "pcm",
        "speed": super::speed_param(speed, 0.25, 4.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_raw_pcm_with_clamped_speed() {
        let b = body("gpt-4o-mini-tts", "alloy", 9.0, "Hello.");
        assert_eq!(b["response_format"], "pcm");
        assert_eq!(b["input"], "Hello.");
        assert_eq!(b["speed"], 4.0);
    }
}
