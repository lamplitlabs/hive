//! The recognizer in memory: ONNX Runtime loaded from the downloaded
//! library (once per process - the runtime allows one environment), the
//! model's sessions built on the first request and dropped again after a
//! while idle, since a 0.6B model holds about 2.5 GB.

use crate::audio;
use crate::catalogue::{self, ModelSpec};
use crate::paths::Paths;
use anyhow::{Context as _, Result, anyhow, bail};
use parakeet_rs::{ExecutionConfig, Nemotron, NemotronHandle, NemotronMode};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What one transcription came back with.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Transcript {
    pub text: String,
    pub model: String,
    /// The language tag the model was told, `auto` when it chose, empty for
    /// the English model.
    pub language: String,
    pub seconds: f32,
    pub took_ms: u64,
}

struct Loaded {
    spec: &'static ModelSpec,
    handle: NemotronHandle,
    last_used: Instant,
}

/// The engine the hub and the command line share: the model it holds (if
/// any) and when it was last used.
#[derive(Clone, Default)]
pub struct Engine {
    inner: Arc<Mutex<Option<Loaded>>>,
}

static RUNTIME: OnceLock<std::result::Result<parakeet_rs::ort::environment::Environment, String>> =
    OnceLock::new();

/// Threads the encoder may use: the model is memory-bound, four is what
/// parakeet-rs defaults to and what a laptop gives without starving the hub.
const INTRA_THREADS: usize = 4;

impl Engine {
    /// Loads ONNX Runtime from `library` (first call only; later calls get
    /// the same environment whatever path they name, so the runtime folder
    /// must not change under a running hub).
    fn environment(
        library: &std::path::Path,
    ) -> Result<parakeet_rs::ort::environment::Environment> {
        let env = RUNTIME.get_or_init(|| {
            parakeet_rs::ort::init_from(library)
                .map_err(|e| format!("loading ONNX Runtime from {}: {e}", library.display()))
                .and_then(|builder| {
                    builder
                        .with_name("pulse-voice")
                        .with_telemetry(false)
                        .build()
                        .map_err(|e| format!("starting ONNX Runtime: {e}"))
                })
        });
        env.clone().map_err(|e| anyhow!(e))
    }

    /// Makes sure `spec` is the model in memory, loading it from `paths`
    /// (the files must be set up already: see [`crate::setup::status`]).
    pub fn load(&self, paths: &Paths, spec: &'static ModelSpec) -> Result<()> {
        let mut slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cur) = slot.as_mut()
            && cur.spec.id == spec.id
        {
            cur.last_used = Instant::now();
            return Ok(());
        }
        // Another model asked for: drop the old one before the new one
        // takes its 2.5 GB.
        *slot = None;
        let runtime = catalogue::runtime().ok_or_else(|| {
            anyhow!(
                "no ONNX Runtime build for {} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
        let library = paths.runtime_library(runtime);
        if !library.is_file() {
            bail!(
                "ONNX Runtime is not set up yet ({} is missing); run `pulse voice setup`",
                library.display()
            );
        }
        let dir = paths.model_dir(spec);
        for f in spec.files {
            if !dir.join(f.name).is_file() {
                bail!(
                    "the {} model is not set up yet ({} is missing); run `pulse voice setup`",
                    spec.id,
                    dir.join(f.name).display()
                );
            }
        }
        let env = Self::environment(&library)?;
        let config = ExecutionConfig::new()
            .with_intra_threads(INTRA_THREADS)
            .with_inter_threads(1)
            .with_environment(env);
        let handle = NemotronHandle::from_pretrained(&dir, Some(config))
            .map_err(|e| anyhow!("loading the {} model from {}: {e}", spec.id, dir.display()))?;
        *slot = Some(Loaded {
            spec,
            handle,
            last_used: Instant::now(),
        });
        Ok(())
    }

    /// Transcribes mono 16 kHz samples with the model in memory (loaded
    /// now when it is not). `language` is `voice.language`: `auto` or a
    /// tag for the multilingual model, ignored by the English one.
    pub fn transcribe(
        &self,
        paths: &Paths,
        spec: &'static ModelSpec,
        language: &str,
        samples: &[f32],
    ) -> Result<Transcript> {
        self.load(paths, spec)?;
        let started = Instant::now();
        let mut slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let loaded = slot
            .as_mut()
            .ok_or_else(|| anyhow!("the model was unloaded while the request waited"))?;
        let mut decoder = Nemotron::from_shared(&loaded.handle);
        let language = match decoder.mode() {
            NemotronMode::EnglishOnly => String::new(),
            NemotronMode::Multilingual => {
                let tag = if language.trim().is_empty() {
                    "auto"
                } else {
                    catalogue::language_tag(language).ok_or_else(|| {
                        anyhow!(
                            "voice.language is {:?}, which the model has no prompt for; pick a tag such as en-US or auto",
                            language.trim()
                        )
                    })?
                };
                decoder
                    .set_target_lang(tag)
                    .map_err(|e| anyhow!("setting the language to {tag}: {e}"))?;
                tag.to_string()
            }
        };
        let text = decoder
            .transcribe_audio(&audio::with_margins(samples))
            .map_err(|e| anyhow!("transcribing: {e}"))
            .context("the model failed on this recording")?;
        loaded.last_used = Instant::now();
        Ok(Transcript {
            text: text.trim().to_string(),
            model: loaded.spec.id.to_string(),
            language,
            seconds: audio::seconds(samples),
            took_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// The id of the model in memory, if any.
    pub fn loaded(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|l| l.spec.id.to_string())
    }

    /// How long the model in memory has gone unused.
    pub fn idle(&self) -> Option<Duration> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|l| l.last_used.elapsed())
    }

    /// Drops the model when nothing used it for `after`; returns whether
    /// it did.
    pub fn release_idle(&self, after: Duration) -> bool {
        let mut slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_ref() {
            Some(l) if l.last_used.elapsed() >= after => {
                *slot = None;
                true
            }
            _ => false,
        }
    }

    /// Drops the model now.
    pub fn unload(&self) {
        *self.inner.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_engine_starts_empty_and_refuses_a_model_that_is_not_set_up() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_str().unwrap());
        let engine = Engine::default();
        assert_eq!(engine.loaded(), None);
        assert_eq!(engine.idle(), None);
        assert!(!engine.release_idle(Duration::ZERO));
        let spec = catalogue::model_or_default("");
        let err = engine.load(&paths, spec).unwrap_err().to_string();
        if catalogue::runtime().is_some() {
            assert!(err.contains("ONNX Runtime is not set up yet"), "{err}");
            assert!(err.contains("pulse voice setup"), "{err}");
            // With the library in place but no model, the model is what is named.
            let rt = catalogue::runtime().unwrap();
            let lib = paths.runtime_library(rt);
            std::fs::create_dir_all(lib.parent().unwrap()).unwrap();
            std::fs::write(&lib, b"not a library").unwrap();
            let err = engine.load(&paths, spec).unwrap_err().to_string();
            assert!(err.contains("model is not set up yet"), "{err}");
        } else {
            assert!(err.contains("no ONNX Runtime build"), "{err}");
        }
        engine.unload();
        assert_eq!(engine.loaded(), None);
    }
}
