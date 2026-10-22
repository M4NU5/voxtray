//! ElevenLabs text-to-speech (`/v1/text-to-speech/{voice_id}`).

use crate::settings::{self, Provider, Settings};
use crate::timeline::Pcm;
use serde_json::{json, Value};
use std::time::Duration;

/// Raw signed 16-bit little-endian mono. 24 kHz works on every plan; 44.1 kHz
/// PCM needs Pro.
const OUTPUT_FORMAT: &str = "pcm_24000";
const PCM_RATE: u32 = 24_000;

/// One client per reading, so chunks reuse the same connection.
pub struct Client {
    http: reqwest::blocking::Client,
    key: String,
    url: String,
    model: String,
    speed: f32,
}

impl Client {
    pub fn new(settings: &Settings) -> Result<Client, String> {
        let key = settings::get_api_key(Provider::Elevenlabs)
            .filter(|k| !k.trim().is_empty())
            .ok_or("No ElevenLabs API key set. Add one in settings.")?;

        let voice_id = settings.elevenlabs_voice_id.trim();
        if voice_id.is_empty() {
            return Err("No ElevenLabs voice ID set. Add one in settings.".into());
        }

        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Client {
            http,
            key,
            url: url(voice_id),
            model: settings.elevenlabs_model.clone(),
            speed: settings.speed,
        })
    }

    pub fn synthesize(&self, text: &str) -> Result<Pcm, String> {
        let resp = self
            .http
            .post(&self.url)
            .header("xi-api-key", &self.key)
            .json(&body(&self.model, self.speed, text))
            .send()
            .map_err(|e| format!("request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!(
                "ElevenLabs error {status}: {}",
                super::truncate(&text, 300)
            ));
        }

        let bytes = resp.bytes().map_err(|e| e.to_string())?;
        Ok(Pcm::from_s16le(&bytes, PCM_RATE, 1))
    }
}

fn url(voice_id: &str) -> String {
    format!("https://api.elevenlabs.io/v1/text-to-speech/{voice_id}?output_format={OUTPUT_FORMAT}")
}

fn body(model: &str, speed: f32, text: &str) -> Value {
    json!({
        "text": text,
        "model_id": model,
        "voice_settings": {
            "stability": 0.5,
            "similarity_boost": 0.75,
            // ElevenLabs accepts 0.7 - 1.2.
            "speed": super::speed_param(speed, 0.7, 1.2),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_24k_pcm_with_clamped_speed() {
        assert!(url("abc").ends_with("/abc?output_format=pcm_24000"));
        let b = body("eleven_multilingual_v2", 2.0, "Hello.");
        assert_eq!(b["text"], "Hello.");
        assert_eq!(b["voice_settings"]["speed"], 1.2);
    }
}
