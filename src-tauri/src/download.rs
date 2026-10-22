//! Downloading and unpacking the free local-neural voices: the Piper engine
//! and voices, and the Kokoro model.
//!
//! Nothing is bundled in the installer except the Kokoro engine binary; the
//! Piper engine and the selected voice or model are fetched on first use into
//! the platform data directory and then reused offline.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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

/// Progress for the settings panel: `hd-progress` for Piper, `kokoro-progress`
/// for Kokoro.
fn emit(app: &AppHandle, event: &str, message: &str, pct: Option<u64>) {
    let _ = app.emit(event, json!({ "message": message, "pct": pct }));
}

fn download_file(app: &AppHandle, event: &str, url: &str, dest: &Path, label: &str) -> Result<(), String> {
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
                emit(app, event, &format!("Downloading {label}… {pct}%"), Some(pct));
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

    emit(app, "hd-progress", "Downloading voice engine…", None);
    download_file(app, "hd-progress", &url, &tmp, "engine")?;

    emit(app, "hd-progress", "Extracting engine…", None);
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

    emit(app, "hd-progress", "Downloading voice…", None);
    download_file(app, "hd-progress", &format!("{base}.onnx"), &onnx, "voice")?;
    download_file(app, "hd-progress", &format!("{base}.onnx.json"), &json, "voice config")?;
    Ok(onnx)
}

// --- Kokoro ------------------------------------------------------------------

/// Kokoro v1.0 packaged for sherpa-onnx: fp32, 350 MB. The int8 build is
/// smaller but slower on x86, too slow to stay ahead of playback.
const KOKORO_MODEL: &str = "kokoro-multi-lang-v1_0";
const KOKORO_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_0.tar.bz2";
/// What the engine needs from the archive. The Chinese lexicon and
/// dictionaries are skipped.
const KOKORO_FILES: &[&str] = &[
    "model.onnx", "voices.bin", "tokens.txt", "lexicon-us-en.txt", "lexicon-gb-en.txt",
    "LICENSE", "README.md", "espeak-ng-data/",
];

/// One Kokoro download at a time: the settings button and a reading can
/// both ask for it.
static KOKORO_LOCK: Mutex<()> = Mutex::new(());

/// Where the unpacked model lives.
pub fn kokoro_model_dir() -> Result<PathBuf, String> {
    Ok(data_root()?.join("kokoro").join(KOKORO_MODEL))
}

pub fn kokoro_model_installed() -> bool {
    kokoro_model_dir().map(|d| d.join("model.onnx").exists()).unwrap_or(false)
}

/// Ensure the Kokoro model is unpacked, downloading it if needed. Returns
/// its folder. A download, whether the settings panel or a reading asked
/// for it, ends with `kokoro-done`.
pub fn ensure_kokoro_model(app: &AppHandle) -> Result<PathBuf, String> {
    let _one_at_a_time = KOKORO_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = kokoro_model_dir()?;
    if dir.join("model.onnx").exists() {
        return Ok(dir);
    }
    let started = std::time::Instant::now();
    let result = download_kokoro(app, &dir);
    match &result {
        Ok(()) => log::info!("Kokoro model downloaded in {} s", started.elapsed().as_secs()),
        Err(e) => log::warn!("Kokoro model download failed: {}", crate::logging::loggable(e)),
    }
    let done = match &result {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e }),
    };
    let _ = app.emit("kokoro-done", done);
    result.map(|()| dir)
}

fn download_kokoro(app: &AppHandle, dir: &Path) -> Result<(), String> {
    let parent = dir.parent().ok_or("Kokoro folder not found")?;
    let archive = parent.join(format!("{KOKORO_MODEL}.tar.bz2"));

    emit(app, "kokoro-progress", "Downloading Kokoro model…", None);
    download_file(app, "kokoro-progress", KOKORO_URL, &archive, "Kokoro model")?;
    // The whole archive is on disk before anything is unpacked, so a SHA-256
    // check (roadmap step 4) can go here.

    emit(app, "kokoro-progress", "Unpacking Kokoro model…", None);
    // Unpack beside the final folder, then rename, so a half-unpacked model
    // never looks installed.
    let staging = parent.join(format!("{KOKORO_MODEL}.part"));
    let _ = fs::remove_dir_all(&staging);
    let unpacked = extract_kokoro(&archive, &staging).and_then(|()| {
        fs::rename(staging.join(KOKORO_MODEL), dir).map_err(|e| format!("install Kokoro model: {e}"))
    });
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_file(&archive);
    unpacked
}

/// Unpack the files the engine needs from the model archive into `dest`,
/// keeping the archive's top folder.
fn extract_kokoro(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(file));
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let entries = tar.entries().map_err(|e| format!("read Kokoro archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("read Kokoro archive: {e}"))?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        if kokoro_wants(&path) {
            // unpack_in refuses paths that would land outside `dest`.
            entry.unpack_in(dest).map_err(|e| format!("extract Kokoro model: {e}"))?;
        }
    }
    if !dest.join(KOKORO_MODEL).join("model.onnx").exists() {
        return Err("Kokoro archive did not contain the model.".into());
    }
    Ok(())
}

/// Whether an archive path (`kokoro-multi-lang-v1_0/…`) is one we keep.
fn kokoro_wants(path: &Path) -> bool {
    let Ok(inside) = path.strip_prefix(KOKORO_MODEL) else { return false };
    let inside = inside.to_string_lossy().replace('\\', "/");
    KOKORO_FILES.iter().any(|f| match f.strip_suffix('/') {
        Some(dir) => inside == dir || inside.starts_with(f),
        None => inside == *f,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kokoro_keeps_only_what_the_engine_needs() {
        let wants = |p: &str| kokoro_wants(Path::new(p));
        assert!(wants("kokoro-multi-lang-v1_0/model.onnx"));
        assert!(wants("kokoro-multi-lang-v1_0/espeak-ng-data"));
        assert!(wants("kokoro-multi-lang-v1_0/espeak-ng-data/en_dict"));
        assert!(wants("kokoro-multi-lang-v1_0/lexicon-gb-en.txt"));
        assert!(!wants("kokoro-multi-lang-v1_0/lexicon-zh.txt"));
        assert!(!wants("kokoro-multi-lang-v1_0/dict/jieba.dict.utf8"));
        assert!(!wants("kokoro-multi-lang-v1_0/espeak-ng-data-extra"));
        assert!(!wants("model.onnx"));
        assert!(!wants("other/model.onnx"));
    }

    /// Unpacks the real archive. `VOXTRAY_KOKORO_ARCHIVE` must point at a
    /// downloaded `kokoro-multi-lang-v1_0.tar.bz2`.
    #[test]
    #[ignore = "needs the Kokoro model archive"]
    fn kokoro_archive_unpacks_to_a_usable_model() {
        let archive = std::env::var_os("VOXTRAY_KOKORO_ARCHIVE").expect("set VOXTRAY_KOKORO_ARCHIVE");
        let dest = std::env::temp_dir().join(format!("voxtray-kokoro-unpack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dest);
        extract_kokoro(Path::new(&archive), &dest).unwrap();
        let model = dest.join(KOKORO_MODEL);
        for f in KOKORO_FILES {
            assert!(model.join(f.trim_end_matches('/')).exists(), "{f}");
        }
        assert!(!model.join("dict").exists());
        assert!(!model.join("lexicon-zh.txt").exists());
        fs::remove_dir_all(dest).unwrap();
    }
}
