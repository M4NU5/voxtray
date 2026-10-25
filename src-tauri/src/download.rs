//! Downloading and unpacking the free local-neural voices: the Piper engine
//! and voices, and the Kokoro model.
//!
//! Nothing is bundled in the installer except the Kokoro engine binary; the
//! Piper engine and the selected voice or model are fetched on first use into
//! the platform data directory and then reused offline. Every file is pinned
//! to a SHA-256, checked before it is installed.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

/// Pinned Piper release used for the engine binaries.
const PIPER_RELEASE: &str = "2023.11.14-2";

/// Pinned piper-voices commit, so the voice files can't change under their
/// hashes.
const PIPER_VOICES_REVISION: &str = "5041a7a49ae5b3a0f30e6837769eb03fdbb231bd";

/// A downloadable Piper voice.
pub struct PiperVoice {
    pub key: &'static str,
    pub name: &'static str,
    /// Path prefix under the piper-voices repo, e.g. `en/en_US/amy/medium`.
    pub path: &'static str,
    /// SHA-256 of `{key}.onnx` and `{key}.onnx.json`.
    pub onnx_sha256: &'static str,
    pub json_sha256: &'static str,
}

/// Small curated catalog of good-quality voices across a few languages.
pub const PIPER_VOICES: &[PiperVoice] = &[
    PiperVoice {
        key: "en_US-amy-medium",
        name: "English (US) — Amy · female",
        path: "en/en_US/amy/medium",
        onnx_sha256: "b3a6e47b57b8c7fbe6a0ce2518161a50f59a9cdd8a50835c02cb02bdd6206c18",
        json_sha256: "95a23eb4d42909d38df73bb9ac7f45f597dbfcde2d1bf9526fdeaf5466977d77",
    },
    PiperVoice {
        key: "en_US-hfc_male-medium",
        name: "English (US) — HFC · male",
        path: "en/en_US/hfc_male/medium",
        onnx_sha256: "d11e403a02bdf5a670c877b3dc56e0e1c8cece6fb30289586314dffdc0a78cb0",
        json_sha256: "f66847424aed0bf99ecbb5d7cfde47c0a906f426a0daf7c46f305e7d21afd886",
    },
    PiperVoice {
        key: "en_US-lessac-medium",
        name: "English (US) — Lessac · neutral",
        path: "en/en_US/lessac/medium",
        onnx_sha256: "5efe09e69902187827af646e1a6e9d269dee769f9877d17b16b1b46eeaaf019f",
        json_sha256: "efe19c417bed055f2d69908248c6ba650fa135bc868b0e6abb3da181dab690a0",
    },
    PiperVoice {
        key: "en_GB-alan-medium",
        name: "English (UK) — Alan · male",
        path: "en/en_GB/alan/medium",
        onnx_sha256: "0a309668932205e762801f1efc2736cd4b0120329622adf62be09e56339d3330",
        json_sha256: "c0f0d124e5895c00e7c03b35dcc8287f319a6998a365b182deb5c8e752ee8c1e",
    },
    PiperVoice {
        key: "en_GB-jenny_dioco-medium",
        name: "English (UK) — Jenny · female",
        path: "en/en_GB/jenny_dioco/medium",
        onnx_sha256: "469c630d209e139dd392a66bf4abde4ab86390a0269c1e47b4e5d7ce81526b01",
        json_sha256: "a9a7a93a317c9a3cb6563e37eb057df9ef09c06188a8a4341b0fcb58cba54dd4",
    },
    PiperVoice {
        key: "de_DE-thorsten-medium",
        name: "German — Thorsten · male",
        path: "de/de_DE/thorsten/medium",
        onnx_sha256: "7e64762d8e5118bb578f2eea6207e1a35a8e0c30595010b666f983fc87bb7819",
        json_sha256: "974adee790533adb273a1ac88f49027d2a1b8f0f2cf4905954a4791e79264e85",
    },
    PiperVoice {
        key: "es_ES-sharvard-medium",
        name: "Spanish — Sharvard",
        path: "es/es_ES/sharvard/medium",
        onnx_sha256: "40febfb1679c69a4505ff311dc136e121e3419a13a290ef264fdf43ddedd0fb1",
        json_sha256: "7438c9b699c72b0c3388dae1b68d3f364dc66a2150fe554a1c11f03372957b2c",
    },
    PiperVoice {
        key: "fr_FR-siwis-medium",
        name: "French — Siwis · female",
        path: "fr/fr_FR/siwis/medium",
        onnx_sha256: "641d1ab097da2b81128c076810edb052b385decc8be3381814802a64a73baf99",
        json_sha256: "39479916c2db192b5ac9764daddd0c744d83e023ad890c6976c0633ae4df8959",
    },
    PiperVoice {
        key: "hi_IN-priyamvada-medium",
        name: "Hindi — Priyamvada · female",
        path: "hi/hi_IN/priyamvada/medium",
        onnx_sha256: "aa63bcf2cd493b55a450f280e23cf77f03afc9af7015e6e5acd43b652f166c88",
        json_sha256: "5efc0ccf7529f3528996d46e0fac1f969f681d44a8e55bfa6236ff8841b5d52d",
    },
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

/// The Piper engine archive for this platform, and its SHA-256.
fn archive_name() -> Result<(&'static str, &'static str), String> {
    #[cfg(target_os = "windows")]
    {
        // No native Windows ARM64 build; the amd64 binary runs under emulation.
        Ok(("piper_windows_amd64.zip", "f3c58906402b24f3a96d92145f58acba6d86c9b5db896d207f78dc80811efcea"))
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Ok(("piper_macos_x64.tar.gz", "ced85c0a3df13945b1e623b878a48fdc2854d5c485b4b67f62857cf551deaf8b"))
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Ok(("piper_macos_aarch64.tar.gz", "6b1eb03b3735946cb35216e063e7eebcc33a6bbf5dd96ec0217959bf1cdcb0cc"))
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Ok(("piper_linux_x86_64.tar.gz", "a50cb45f355b7af1f6d758c1b360717877ba0a398cc8cbe6d2a7a3a26e225992"))
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        Ok(("piper_linux_aarch64.tar.gz", "fea0fd2d87c54dbc7078d0f878289f404bd4d6eea6e7444a77835d1537ab88eb"))
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

/// Download `url` to `dest`. The file only lands at `dest` if its SHA-256 is
/// `sha256`, so a corrupt or swapped download is never installed.
fn download_file(
    app: &AppHandle,
    event: &str,
    url: &str,
    dest: &Path,
    label: &str,
    sha256: &str,
) -> Result<(), String> {
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

    let mut last_pct = 0u64;
    let copied = copy_checked(&mut resp, &mut file, label, sha256, |downloaded| {
        if let Some(total) = total {
            let pct = downloaded * 100 / total.max(1);
            if pct != last_pct {
                last_pct = pct;
                emit(app, event, &format!("Downloading {label}… {pct}%"), Some(pct));
            }
        }
    });
    drop(file);
    if let Err(e) = copied {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, dest).map_err(|e| format!("finalize {label}: {e}"))?;
    Ok(())
}

/// Copy `from` into `to`, reporting the bytes copied so far, and fail unless
/// what was copied has the SHA-256 `sha256` (lowercase hex).
fn copy_checked(
    from: &mut impl Read,
    to: &mut impl std::io::Write,
    label: &str,
    sha256: &str,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    let mut hasher = Sha256::new();
    let mut copied: u64 = 0;
    let mut buf = [0u8; 65536];
    loop {
        let n = from.read(&mut buf).map_err(|e| format!("read {label}: {e}"))?;
        if n == 0 {
            break;
        }
        to.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hasher.update(&buf[..n]);
        copied += n as u64;
        progress(copied);
    }
    if format!("{:x}", hasher.finalize()) != sha256 {
        return Err(format!(
            "download {label}: the file isn't the one Voxtray expects (SHA-256 mismatch). Try again later."
        ));
    }
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

    let (archive, sha256) = archive_name()?;
    let url = engine_url(archive);
    let tmp = data_root()?.join(archive);

    emit(app, "hd-progress", "Downloading voice engine…", None);
    download_file(app, "hd-progress", &url, &tmp, "engine", sha256)?;

    emit(app, "hd-progress", "Extracting engine…", None);
    if archive.ends_with(".zip") {
        extract_zip(&tmp, &dir)?;
    } else {
        extract_targz(&tmp, &dir)?;
    }
    let _ = fs::remove_file(&tmp);

    installed_exe().ok_or_else(|| "Piper executable not found after extraction.".into())
}

fn engine_url(archive: &str) -> String {
    format!("https://github.com/rhasspy/piper/releases/download/{PIPER_RELEASE}/{archive}")
}

/// The voice's URL without the `.onnx` / `.onnx.json` extension.
fn voice_url(voice: &PiperVoice) -> String {
    format!(
        "https://huggingface.co/rhasspy/piper-voices/resolve/{PIPER_VOICES_REVISION}/{}/{}",
        voice.path, voice.key
    )
}

/// Ensure the given voice model is present, downloading it if needed.
pub fn ensure_voice(app: &AppHandle, key: &str) -> Result<PathBuf, String> {
    let (onnx, json) = voice_files(key)?;
    if onnx.exists() && json.exists() {
        return Ok(onnx);
    }
    let voice = find_voice(key)?;
    let base = voice_url(voice);

    emit(app, "hd-progress", "Downloading voice…", None);
    download_file(app, "hd-progress", &format!("{base}.onnx"), &onnx, "voice", voice.onnx_sha256)?;
    download_file(app, "hd-progress", &format!("{base}.onnx.json"), &json, "voice config", voice.json_sha256)?;
    Ok(onnx)
}

// --- Kokoro ------------------------------------------------------------------

/// Kokoro v1.0 packaged for sherpa-onnx: fp32, 350 MB. The int8 build is
/// smaller but slower on x86, too slow to stay ahead of playback.
const KOKORO_MODEL: &str = "kokoro-multi-lang-v1_0";
const KOKORO_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_0.tar.bz2";
const KOKORO_SHA256: &str = "c5f7e2d2caf082bc1d20fb70334a61d99d20b484500aad32e7cf84c128ea3298";
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
    let dir = kokoro_model_dir();
    if let Ok(dir) = &dir {
        if dir.join("model.onnx").exists() {
            return Ok(dir.clone());
        }
    }
    let started = std::time::Instant::now();
    // A data folder that can't be created fails here too, so the panel hears.
    let result = dir.and_then(|dir| download_kokoro(app, &dir).map(|()| dir));
    match &result {
        Ok(_) => log::info!("Kokoro model downloaded in {} s", started.elapsed().as_secs()),
        Err(e) => log::warn!("Kokoro model download failed: {}", crate::logging::loggable(e)),
    }
    let done = match &result {
        Ok(_) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e }),
    };
    let _ = app.emit("kokoro-done", done);
    result
}

fn download_kokoro(app: &AppHandle, dir: &Path) -> Result<(), String> {
    let parent = dir.parent().ok_or("Kokoro folder not found")?;
    let archive = parent.join(format!("{KOKORO_MODEL}.tar.bz2"));

    emit(app, "kokoro-progress", "Downloading Kokoro model…", None);
    download_file(app, "kokoro-progress", KOKORO_URL, &archive, "Kokoro model", KOKORO_SHA256)?;

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

    /// SHA-256 of `b"voxtray"`.
    const VOXTRAY_SHA256: &str = "c5a5567ae3da2a74f9664113836c3fe31ff5aac91d7b3f356c066273264917a3";

    #[test]
    fn copy_checked_keeps_a_matching_download() {
        let mut out = Vec::new();
        let mut seen = 0;
        copy_checked(&mut &b"voxtray"[..], &mut out, "test", VOXTRAY_SHA256, |n| seen = n).unwrap();
        assert_eq!(out, b"voxtray");
        assert_eq!(seen, 7);
    }

    #[test]
    fn copy_checked_rejects_a_changed_download() {
        let err = copy_checked(&mut &b"voxtraY"[..], &mut Vec::new(), "voice", VOXTRAY_SHA256, |_| {}).unwrap_err();
        assert!(err.contains("SHA-256 mismatch"), "{err}");
    }

    #[test]
    fn every_download_is_pinned() {
        let is_sha256 = |h: &str| h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        for v in PIPER_VOICES {
            assert!(is_sha256(v.onnx_sha256) && is_sha256(v.json_sha256), "{}", v.key);
        }
        assert!(is_sha256(KOKORO_SHA256));
        if let Ok((_, sha256)) = archive_name() {
            assert!(is_sha256(sha256));
        }
    }

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

    /// Downloads everything this platform can fetch (about 900 MB) and checks
    /// it against the pinned hashes. Run it after bumping a pin.
    #[test]
    #[ignore = "downloads about 900 MB"]
    fn pinned_hashes_match_upstream() {
        let mut files = vec![(KOKORO_URL.to_string(), KOKORO_SHA256)];
        if let Ok((archive, sha256)) = archive_name() {
            files.push((engine_url(archive), sha256));
        }
        for v in PIPER_VOICES {
            files.push((format!("{}.onnx", voice_url(v)), v.onnx_sha256));
            files.push((format!("{}.onnx.json", voice_url(v)), v.json_sha256));
        }
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(1800)).build().unwrap();
        for (url, sha256) in files {
            let mut resp = client.get(&url).send().and_then(|r| r.error_for_status()).unwrap();
            copy_checked(&mut resp, &mut std::io::sink(), &url, sha256, |_| {}).unwrap();
        }
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
