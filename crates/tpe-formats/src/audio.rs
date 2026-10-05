//! Audio transcription through a local speech engine found at run time:
//! whisper.cpp (`whisper-cli`, `whisper-cpp`, or a binary named by
//! `TPE_WHISPER_BIN`) with a local GGML model, or the `whisper` CLI of
//! openai-whisper with a model file already in a local directory. `mp3` and
//! `m4a` are converted to 16 kHz mono WAV with `ffmpeg` first. No engine or
//! no local model means `unsupported here: install ...`; nothing is ever
//! fetched from the network.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Block, DocumentIdentity, Format, FormatsError, FormatsResult, Section};

/// Where to look for an engine. Built from the process environment by
/// [`EngineEnv::from_process`]; tests construct it directly.
#[derive(Clone, Debug, Default)]
pub struct EngineEnv {
    /// The `PATH` to search; `None` searches nothing.
    pub path: Option<OsString>,
    /// `TPE_WHISPER_BIN`: an explicit whisper.cpp binary.
    pub whisper_bin: Option<PathBuf>,
    /// `TPE_WHISPER_MODEL`: a GGML model file for whisper.cpp.
    pub whisper_model: Option<PathBuf>,
    /// `TPE_WHISPER_MODEL_DIR`: a directory holding openai-whisper `.pt` models.
    pub whisper_model_dir: Option<PathBuf>,
    /// `TPE_WHISPER_MODEL_NAME`: the openai-whisper model name (default `base`).
    pub whisper_model_name: Option<String>,
    /// Home directory, for the default whisper.cpp model cache.
    pub home: Option<PathBuf>,
}

impl EngineEnv {
    /// Read the process environment.
    pub fn from_process() -> Self {
        Self {
            path: std::env::var_os("PATH"),
            whisper_bin: std::env::var_os("TPE_WHISPER_BIN").map(PathBuf::from),
            whisper_model: std::env::var_os("TPE_WHISPER_MODEL").map(PathBuf::from),
            whisper_model_dir: std::env::var_os("TPE_WHISPER_MODEL_DIR").map(PathBuf::from),
            whisper_model_name: std::env::var("TPE_WHISPER_MODEL_NAME").ok(),
            home: std::env::var_os("HOME").map(PathBuf::from),
        }
    }

    fn find_in_path(&self, name: &str) -> Option<PathBuf> {
        let path = self.path.as_ref()?;
        std::env::split_paths(path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    }
}

/// A usable local engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Engine {
    /// whisper.cpp binary and GGML model.
    WhisperCpp { binary: PathBuf, model: PathBuf },
    /// openai-whisper CLI with a local model directory and model name.
    OpenAiWhisper {
        binary: PathBuf,
        model_dir: PathBuf,
        model: String,
    },
}

impl Engine {
    /// Label for `backend.engine`.
    pub fn label(&self) -> String {
        match self {
            Self::WhisperCpp { binary, model } => {
                format!("whisper.cpp {} model {}", binary.display(), model.display())
            }
            Self::OpenAiWhisper {
                binary,
                model_dir,
                model,
            } => format!(
                "openai-whisper {} model {model} in {}",
                binary.display(),
                model_dir.display()
            ),
        }
    }
}

const WHISPER_CPP_NAMES: [&str; 3] = ["whisper-cli", "whisper-cpp", "whisper.cpp"];

/// Find an engine, or explain precisely what is missing.
pub fn detect(env: &EngineEnv) -> Result<Engine, String> {
    let mut missing = Vec::new();
    let binary = env
        .whisper_bin
        .clone()
        .filter(|b| b.is_file())
        .or_else(|| WHISPER_CPP_NAMES.iter().find_map(|n| env.find_in_path(n)));
    match binary {
        Some(binary) => match whisper_cpp_model(env) {
            Some(model) => return Ok(Engine::WhisperCpp { binary, model }),
            None => missing.push(format!(
                "whisper.cpp found at {} but no GGML model: set TPE_WHISPER_MODEL=/path/ggml-base.bin (or put one in ~/.cache/whisper.cpp)",
                binary.display()
            )),
        },
        None => missing.push(
            "whisper.cpp not found: install it and put whisper-cli on PATH (or set TPE_WHISPER_BIN)".to_string(),
        ),
    }
    if let Some(binary) = env.find_in_path("whisper") {
        let model = env
            .whisper_model_name
            .clone()
            .unwrap_or_else(|| "base".to_string());
        match env.whisper_model_dir.clone() {
            Some(dir) if dir.join(format!("{model}.pt")).is_file() => {
                return Ok(Engine::OpenAiWhisper {
                    binary,
                    model_dir: dir,
                    model,
                });
            }
            Some(dir) => missing.push(format!(
                "openai-whisper found at {} but {} has no {model}.pt (models are never downloaded here)",
                binary.display(),
                dir.display()
            )),
            None => missing.push(format!(
                "openai-whisper found at {} but TPE_WHISPER_MODEL_DIR is unset (models are never downloaded here)",
                binary.display()
            )),
        }
    } else {
        missing.push("openai-whisper not found (pip-free install: uv tool install openai-whisper, then set TPE_WHISPER_MODEL_DIR)".to_string());
    }
    Err(missing.join("; "))
}

fn whisper_cpp_model(env: &EngineEnv) -> Option<PathBuf> {
    if let Some(model) = env.whisper_model.clone().filter(|m| m.is_file()) {
        return Some(model);
    }
    let mut dirs = Vec::new();
    if let Some(home) = &env.home {
        dirs.push(home.join(".cache/whisper.cpp"));
        dirs.push(home.join(".cache/whisper.cpp/models"));
    }
    dirs.push(PathBuf::from("/usr/share/whisper.cpp/models"));
    dirs.push(PathBuf::from("/usr/local/share/whisper.cpp/models"));
    dirs.push(PathBuf::from("/opt/homebrew/share/whisper-cpp/models"));
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut models: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|e| e == "bin")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("ggml-"))
            })
            .collect();
        models.sort();
        if let Some(model) = models.into_iter().next() {
            return Some(model);
        }
    }
    None
}

/// Basic facts from a RIFF/WAVE header: sample rate, channels, seconds.
pub fn wav_facts(bytes: &[u8]) -> Option<(u32, u16, f64)> {
    if !(bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE")) {
        return None;
    }
    let mut pos = 12;
    let mut rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = pos + 8;
        if id == b"fmt " && body + 16 <= bytes.len() {
            channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().ok()?);
            bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
        } else if id == b"data" {
            let bytes_per_second = f64::from(rate) * f64::from(channels) * f64::from(bits) / 8.0;
            if bytes_per_second <= 0.0 {
                return None;
            }
            let data_len = size.min(bytes.len().saturating_sub(body));
            return Some((rate, channels, data_len as f64 / bytes_per_second));
        }
        pos = body + size + (size % 2);
    }
    None
}

/// Transcribe `path` (bytes already read as `bytes`), or report what is missing.
pub fn extract(
    path: &Path,
    bytes: &[u8],
    identity: DocumentIdentity,
    env: &EngineEnv,
) -> Result<FormatsResult, FormatsError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let mut result = match detect(env) {
        Ok(engine) => {
            let mut result = FormatsResult::empty(Format::Audio, identity);
            result.backend.engine = Some(engine.label());
            let wav = prepare_wav(path, &ext, env)?;
            match wav {
                Ok(wav) => {
                    let text = run_engine(&engine, &wav.path)?;
                    let mut section = Section::new("transcript", 0, None);
                    for paragraph in text.split("\n\n") {
                        let paragraph = paragraph.trim();
                        if !paragraph.is_empty() {
                            section.blocks.push(Block::paragraph(paragraph));
                        }
                    }
                    if section.blocks.is_empty() {
                        result.warn("partial: the engine produced an empty transcript");
                    }
                    result.sections.push(section);
                    result
                }
                Err(reason) => FormatsResult::unsupported(Format::Audio, result.document, &reason),
            }
        }
        Err(reason) => FormatsResult::unsupported(
            Format::Audio,
            identity,
            &format!("unsupported here: no local speech engine. {reason}"),
        ),
    };
    result.metadata.insert("container".to_string(), ext.clone());
    if let Some((rate, channels, seconds)) = wav_facts(bytes) {
        result
            .metadata
            .insert("sample_rate".to_string(), rate.to_string());
        result
            .metadata
            .insert("channels".to_string(), channels.to_string());
        result
            .metadata
            .insert("duration_seconds".to_string(), format!("{seconds:.2}"));
    }
    Ok(result)
}

/// A WAV ready for the engine; temporary files are removed on drop.
struct PreparedWav {
    path: PathBuf,
    _temp: Option<tempfile_dir::TempDir>,
}

mod tempfile_dir {
    //! Minimal scoped temporary directory (no extra dependency at run time).
    use std::path::{Path, PathBuf};

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(label: &str) -> std::io::Result<Self> {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let dir = std::env::temp_dir().join(format!(
                "tpe-formats-{label}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// `Ok(Ok(wav))` when the input is usable, `Ok(Err(reason))` when a converter
/// is missing, `Err` for I/O failures.
fn prepare_wav(
    path: &Path,
    ext: &str,
    env: &EngineEnv,
) -> Result<Result<PreparedWav, String>, FormatsError> {
    if ext == "wav" {
        return Ok(Ok(PreparedWav {
            path: path.to_path_buf(),
            _temp: None,
        }));
    }
    let Some(ffmpeg) = env.find_in_path("ffmpeg") else {
        return Ok(Err(format!(
            "unsupported here: {ext} needs ffmpeg to convert to 16 kHz WAV; install ffmpeg"
        )));
    };
    let temp = tempfile_dir::TempDir::new("audio")?;
    let wav = temp.path().join("input.wav");
    let status = Command::new(ffmpeg)
        .args(["-nostdin", "-loglevel", "error", "-y", "-i"])
        .arg(path)
        .args(["-ar", "16000", "-ac", "1", "-f", "wav"])
        .arg(&wav)
        .status()?;
    if !status.success() {
        return Err(FormatsError::Invalid(format!(
            "ffmpeg could not decode {} (exit {status})",
            path.display()
        )));
    }
    Ok(Ok(PreparedWav {
        path: wav,
        _temp: Some(temp),
    }))
}

fn run_engine(engine: &Engine, wav: &Path) -> Result<String, FormatsError> {
    match engine {
        Engine::WhisperCpp { binary, model } => {
            let output = Command::new(binary)
                .arg("-m")
                .arg(model)
                .arg("-f")
                .arg(wav)
                .args(["-nt", "-np"])
                .output()?;
            if !output.status.success() {
                return Err(FormatsError::Invalid(format!(
                    "{} failed ({}): {}",
                    binary.display(),
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        Engine::OpenAiWhisper {
            binary,
            model_dir,
            model,
        } => {
            let temp = tempfile_dir::TempDir::new("whisper")?;
            let status = Command::new(binary)
                .arg(wav)
                .args(["--model", model, "--model_dir"])
                .arg(model_dir)
                .args(["--output_format", "txt", "--output_dir"])
                .arg(temp.path())
                .args(["--verbose", "False"])
                .status()?;
            if !status.success() {
                return Err(FormatsError::Invalid(format!(
                    "{} failed ({status})",
                    binary.display()
                )));
            }
            let stem = wav.file_stem().and_then(|s| s.to_str()).unwrap_or("input");
            let text = std::fs::read_to_string(temp.path().join(format!("{stem}.txt")))?;
            Ok(text.trim().to_string())
        }
    }
}
