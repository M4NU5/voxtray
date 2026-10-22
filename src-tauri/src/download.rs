//! Downloading and unpacking the free local-neural (Piper) engine and voices.
//!
//! Nothing is bundled in the installer; the engine binary and the selected
//! voice model are fetched on first use into the platform data directory and
//! then reused offline.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Emitter};

/// Pinned Piper release used for the engine binaries.
const PIPER_RELEASE: &str = "2023.11.14-2";

/// A downloadable Piper voice.
pub struct PiperVoice {
    pub key: &'static str,
    pub name: &'static str,
    /// Path prefix under the piper-voices repo, e.g. `en/en_US/amy/medium`.
    pub path: &'static str,
}

/// Small curated catalog of good-quality voices across a few languages.
pub const PIPER_VOICES: &[PiperVoice] = &[
    PiperVoice { key: "en_US-amy-medium", name: "English (US) — Amy · female", path: "en/en_US/amy/medium" },
    PiperVoice { key: "en_US-hfc_male-medium", name: "English (US) — HFC · male", path: "en/en_US/hfc_male/medium" },
    PiperVoice { key: "en_US-lessac-medium", name: "English (US) — Lessac · neutral", path: "en/en_US/lessac/medium" },
    PiperVoice { key: "en_GB-alan-medium", name: "English (UK) — Alan · male", path: "en/en_GB/alan/medium" },
    PiperVoice { key: "en_GB-jenny_dioco-medium", name: "English (UK) — Jenny · female", path: "en/en_GB/jenny_dioco/medium" },
    PiperVoice { key: "de_DE-thorsten-medium", name: "German — Thorsten · male", path: "de/de_DE/thorsten/medium" },
    PiperVoice { key: "es_ES-sharvard-medium", name: "Spanish — Sharvard", path: "es/es_ES/sharvard/medium" },
    PiperVoice { key: "fr_FR-siwis-medium", name: "French — Siwis · female", path: "fr/fr_FR/siwis/medium" },
    PiperVoice { key: "hi_IN-priyamvada-medium", name: "Hindi — Priyamvada · female", path: "hi/hi_IN/priyamvada/medium" },
];

pub fn find_voice(key: &str) -> Result<&'static PiperVoice, String> {
    PIPER_VOICES
        .iter()
        .find(|v| v.key == key)
        .ok_or_else(|| format!("unknown Piper voice: {key}"))
}

// --- Paths -------------------------------------------------------------------

fn base_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| {
            let mut p = PathBuf::from(h);
            p.push("Library/Application Support");
            p
        })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
            Some(PathBuf::from(x))
        } else {
            std::env::var_os("HOME").map(|h| {
                let mut p = PathBuf::from(h);
                p.push(".local/share");
                p
            })
        }
    }
}

fn data_root() -> Result<PathBuf, String> {
    let mut d = base_data_dir().ok_or("could not resolve a data directory")?;
    d.push(crate::settings::app_id());
    fs::create_dir_all(&d).map_err(|e| format!("create data dir: {e}"))?;
    Ok(d)
}

fn engine_dir() -> Result<PathBuf, String> {
    Ok(data_root()?.join("piper").join("engine"))
}

fn voices_dir() -> Result<PathBuf, String> {
    let d = data_root()?.join("piper").join("voices");
    fs::create_dir_all(&d).map_err(|e| format!("create voices dir: {e}"))?;
    Ok(d)
}

/// `(model.onnx, model.onnx.json)` paths for a voice key.
pub fn voice_files(key: &str) -> Result<(PathBuf, PathBuf), String> {
    let dir = voices_dir()?;
    Ok((dir.join(format!("{key}.onnx")), dir.join(format!("{key}.onnx.json"))))
}

fn archive_name() -> Result<&'static str, String> {
    #[cfg(target_os = "windows")]
    {
        // No native Windows ARM64 build; the amd64 binary runs under emulation.
        Ok("piper_windows_amd64.zip")
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Ok("piper_macos_x64.tar.gz")
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Ok("piper_macos_aarch64.tar.gz")
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Ok("piper_linux_x86_64.tar.gz")
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        Ok("piper_linux_aarch64.tar.gz")
    }
    #[cfg(not(any(
        target_os = "windows",
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    {
        Err("No Piper build is available for this platform/architecture.".into())
    }
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "piper.exe"
    } else {
        "piper"
    }
}

/// Recursively search `dir` for the piper executable.
fn find_exe(dir: &Path) -> Option<PathBuf> {
    let want = exe_name();
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_exe(&path) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(want) {
            return Some(path);
        }
    }
    None
}

/// Path to the installed piper executable, if present.
pub fn installed_exe() -> Option<PathBuf> {
    let dir = engine_dir().ok()?;
    find_exe(&dir)
}

pub fn voice_installed(key: &str) -> bool {
    match voice_files(key) {
        Ok((onnx, json)) => onnx.exists() && json.exists(),
        Err(_) => false,
    }
}

// --- Download ----------------------------------------------------------------

fn emit(app: &AppHandle, message: &str, pct: Option<u64>) {
    let _ = app.emit("hd-progress", json!({ "message": message, "pct": pct }));
}

fn download_file(app: &AppHandle, url: &str, dest: &Path, label: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(1800))
        .build()
        .map_err(|e| e.to_string())?;

    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| format!("download {label}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download {label}: HTTP {}", resp.status()));
    }

    let total = resp.content_length();
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Download to a temp file, then rename, so an interrupted download can't
    // leave a half-written model that looks valid.
    let tmp = dest.with_extension("part");
    let mut file = fs::File::create(&tmp).map_err(|e| format!("create {label}: {e}"))?;

    let mut downloaded: u64 = 0;
    let mut buf = [0u8; 65536];
    let mut last_pct = 0u64;
    loop {
        let n = resp.read(&mut buf).map_err(|e| format!("read {label}: {e}"))?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;
        if let Some(total) = total {
            let pct = downloaded * 100 / total.max(1);
            if pct != last_pct {
                last_pct = pct;
                emit(app, &format!("Downloading {label}… {pct}%"), Some(pct));
            }
        }
    }
    drop(file);
    fs::rename(&tmp, dest).map_err(|e| format!("finalize {label}: {e}"))?;
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let out = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue,
        };
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut outfile = fs::File::create(&out).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut outfile).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn extract_targz(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    tar.unpack(dest).map_err(|e| format!("extract engine: {e}"))?;
    Ok(())
}

/// Ensure the Piper engine binary is present, downloading it if needed.
pub fn ensure_engine(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(exe) = installed_exe() {
        return Ok(exe);
    }
    let dir = engine_dir()?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let archive = archive_name()?;
    let url = format!(
        "https://github.com/rhasspy/piper/releases/download/{PIPER_RELEASE}/{archive}"
    );
    let tmp = data_root()?.join(archive);

    emit(app, "Downloading voice engine…", None);
    download_file(app, &url, &tmp, "engine")?;

    emit(app, "Extracting engine…", None);
    if archive.ends_with(".zip") {
        extract_zip(&tmp, &dir)?;
    } else {
        extract_targz(&tmp, &dir)?;
    }
    let _ = fs::remove_file(&tmp);

    installed_exe().ok_or_else(|| "Piper executable not found after extraction.".into())
}

/// Ensure the given voice model is present, downloading it if needed.
pub fn ensure_voice(app: &AppHandle, key: &str) -> Result<PathBuf, String> {
    let (onnx, json) = voice_files(key)?;
    if onnx.exists() && json.exists() {
        return Ok(onnx);
    }
    let voice = find_voice(key)?;
    let base = format!(
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/{}/{}",
        voice.path, voice.key
    );

    emit(app, "Downloading voice…", None);
    download_file(app, &format!("{base}.onnx"), &onnx, "voice")?;
    download_file(app, &format!("{base}.onnx.json"), &json, "voice config")?;
    Ok(onnx)
}
