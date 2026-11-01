//! What the microphone has on this machine and the setup that fetches what
//! is missing: the runtime for this platform, then the chosen model's files
//! one by one. One setup runs at a time per PULSE home (a lock file), and
//! its progress is shared through [`Setup`] so the console can poll it.

use crate::catalogue::{self, ModelSpec, RuntimeSpec};
use crate::download::{self, Progress};
use crate::paths::{Paths, partial, partial_len, sidecar, verified};
use anyhow::{Context as _, Result, anyhow, bail};
use fs2::FileExt;
use serde::Serialize;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// One file's state on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileStatus {
    pub name: String,
    pub size: u64,
    /// Bytes present: the whole file once it is checked, else the part.
    pub have: u64,
    pub ready: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RuntimeStatus {
    pub version: String,
    pub platform: String,
    pub library: String,
    pub size: u64,
    pub have: u64,
    pub ready: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelStatus {
    pub id: String,
    pub title: String,
    pub languages: String,
    pub size: u64,
    pub have: u64,
    pub ready: bool,
    pub files: Vec<FileStatus>,
}

/// A running or finished setup, as the console shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SetupSnapshot {
    /// `runtime`, `model`, `done`, `failed`, `stopped`; empty when none ran.
    pub phase: String,
    pub model: String,
    /// The file being fetched.
    pub file: String,
    /// Bytes done of the whole setup (runtime and model together) and its total.
    pub done: u64,
    pub total: u64,
    pub error: String,
    pub started_at: String,
    pub finished_at: String,
}

/// Everything `GET /api/v1/voice` and `pulse voice status` say.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Status {
    /// Whether a runtime build exists for this machine.
    pub supported: bool,
    pub platform: String,
    pub runtime: Option<RuntimeStatus>,
    /// The model `voice.model` names.
    pub model: ModelStatus,
    pub language: String,
    pub models: Vec<ModelStatus>,
    /// Runtime and chosen model both on disk and checked.
    pub ready: bool,
    /// The model in memory, if any.
    pub loaded: Option<String>,
    pub free_bytes: u64,
    pub setup: Option<SetupSnapshot>,
    pub root: String,
}

fn runtime_status(paths: &Paths, rt: &RuntimeSpec) -> RuntimeStatus {
    let lib = paths.runtime_library(rt);
    let ready = verified(&lib, rt.library_size, rt.library_sha256);
    RuntimeStatus {
        version: rt.version.into(),
        platform: rt.platform.into(),
        library: lib.display().to_string(),
        size: rt.library_size,
        have: if ready { rt.library_size } else { 0 },
        ready,
    }
}

pub fn model_status(paths: &Paths, spec: &ModelSpec) -> ModelStatus {
    let files: Vec<FileStatus> = spec
        .files
        .iter()
        .map(|f| {
            let path = paths.model_file(spec, f);
            let ready = verified(&path, f.size, f.sha256);
            FileStatus {
                name: f.name.into(),
                size: f.size,
                have: if ready {
                    f.size
                } else {
                    partial_len(&path).min(f.size)
                },
                ready,
            }
        })
        .collect();
    ModelStatus {
        id: spec.id.into(),
        title: spec.title.into(),
        languages: spec.languages.into(),
        size: spec.size(),
        have: files.iter().map(|f| f.have).sum(),
        ready: files.iter().all(|f| f.ready),
        files,
    }
}

/// The state of the voice files for `model` (the `voice.model` value) and
/// `language` (`voice.language`), with what the engine holds and the setup
/// in flight.
pub fn status(
    paths: &Paths,
    model: &str,
    language: &str,
    loaded: Option<String>,
    setup: Option<SetupSnapshot>,
) -> Status {
    let spec = catalogue::model_or_default(model);
    let runtime = catalogue::runtime().map(|rt| runtime_status(paths, rt));
    let model = model_status(paths, spec);
    let ready = runtime.as_ref().is_some_and(|r| r.ready) && model.ready;
    let free_bytes = fs2::available_space(&paths.root)
        .or_else(|_| {
            paths
                .root
                .ancestors()
                .find(|a| a.exists())
                .map(fs2::available_space)
                .unwrap_or(Ok(0))
        })
        .unwrap_or(0);
    Status {
        supported: runtime.is_some(),
        platform: catalogue::platform().unwrap_or("").into(),
        runtime,
        model,
        language: if language.trim().is_empty() {
            "auto".into()
        } else {
            language.trim().into()
        },
        models: catalogue::MODELS
            .iter()
            .map(|m| model_status(paths, m))
            .collect(),
        ready,
        loaded,
        free_bytes,
        setup,
        root: paths.root.display().to_string(),
    }
}

/// The progress of one setup, shared between the thread that runs it and
/// whoever reports it. Clone to share.
#[derive(Clone, Default)]
pub struct Setup {
    snapshot: Arc<Mutex<SetupSnapshot>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl Setup {
    pub fn snapshot(&self) -> SetupSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Whether a setup runs right now (this process only; another process's
    /// setup shows as the lock being taken).
    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Asks the setup in flight to stop after the chunk it is writing; the
    /// parts stay for a later resume.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    fn update(&self, f: impl FnOnce(&mut SetupSnapshot)) {
        f(&mut self.snapshot.lock().unwrap_or_else(|e| e.into_inner()));
    }
}

/// Headroom kept on the disk beyond the bytes still to fetch.
const DISK_MARGIN: u64 = 512 * 1024 * 1024;

/// Fetches whatever `model` still needs on this machine - runtime first -
/// reporting through `setup`. Returns Ok when everything is in place; a
/// stop request ends with `phase = stopped` and an Err("stopped").
pub fn run(paths: &Paths, model: &'static ModelSpec, setup: &Setup) -> Result<()> {
    if setup.running.swap(true, Ordering::SeqCst) {
        bail!("a voice setup is already running in this hub");
    }
    setup.stop.store(false, Ordering::SeqCst);
    let now = pulse_store::now;
    setup.update(|s| {
        *s = SetupSnapshot {
            phase: "runtime".into(),
            model: model.id.into(),
            started_at: now(),
            ..Default::default()
        }
    });
    let outcome = run_inner(paths, model, setup);
    setup.update(|s| {
        s.finished_at = now();
        match &outcome {
            Ok(()) => {
                s.phase = "done".into();
                s.file.clear();
                s.done = s.total;
            }
            Err(e) if e.to_string() == "stopped" => s.phase = "stopped".into(),
            Err(e) => {
                s.phase = "failed".into();
                s.error = format!("{e:#}");
            }
        }
    });
    setup.running.store(false, Ordering::SeqCst);
    outcome
}

fn run_inner(paths: &Paths, model: &'static ModelSpec, setup: &Setup) -> Result<()> {
    let runtime = catalogue::runtime().ok_or_else(|| {
        anyhow!(
            "no ONNX Runtime build for {} {}; the microphone needs an Apple-silicon Mac or an x86-64/arm64 Linux machine",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    fs::create_dir_all(&paths.root)
        .with_context(|| format!("creating {}", paths.root.display()))?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.lock_file())?;
    if lock.try_lock_exclusive().is_err() {
        bail!(
            "another voice setup holds {} (a `pulse voice setup` in a terminal, or another hub); wait for it",
            paths.lock_file().display()
        );
    }

    // What is still to fetch, so the progress bar and the disk check see
    // the whole job.
    let rt_status = runtime_status(paths, runtime);
    let model_status = model_status(paths, model);
    let total = rt_status.size + model_status.size;
    let mut base = if rt_status.ready { rt_status.size } else { 0 };
    let need: u64 = (if rt_status.ready {
        0
    } else {
        runtime.archive_size + runtime.library_size
    }) + model_status
        .files
        .iter()
        .map(|f| f.size - f.have)
        .sum::<u64>();
    let free = fs2::available_space(&paths.root).unwrap_or(u64::MAX);
    if free < need + DISK_MARGIN {
        bail!(
            "{} has {} free; the setup needs {} more (the model is {} and the runtime {}), plus room to breathe",
            paths.root.display(),
            size_words(free),
            size_words(need),
            size_words(model.size()),
            size_words(runtime.library_size)
        );
    }
    setup.update(|s| {
        s.total = total;
        s.done = base + model_status.have;
    });
    let client = download::client()?;

    if !rt_status.ready {
        let dir = paths.runtime_dir(runtime);
        fs::create_dir_all(&dir)?;
        let archive = dir.join(format!("onnxruntime-{}.tgz", runtime.platform));
        setup.update(|s| {
            s.phase = "runtime".into();
            s.file = archive
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
        });
        // The library's share of the bar grows with the archive (which is
        // smaller than the library it unpacks to).
        let lib_size = runtime.library_size;
        let arch_size = runtime.archive_size.max(1);
        let model_have = model_status.have;
        let mut report = |p: Progress| {
            setup.update(|s| s.done = (p.done * lib_size / arch_size) + model_have);
            !setup.stop.load(Ordering::SeqCst)
        };
        if !verified(&archive, runtime.archive_size, runtime.archive_sha256) {
            download::fetch(
                &client,
                &runtime.url(),
                &archive,
                runtime.archive_size,
                runtime.archive_sha256,
                &mut report,
            )
            .with_context(|| format!("fetching ONNX Runtime {}", runtime.version))?;
        }
        let library = paths.runtime_library(runtime);
        download::extract_library(&archive, runtime, &library)
            .with_context(|| format!("unpacking ONNX Runtime {}", runtime.version))?;
        let _ = fs::remove_file(&archive);
        let _ = fs::remove_file(sidecar(&archive));
        base = lib_size;
        setup.update(|s| s.done = base + model_have);
    }

    setup.update(|s| s.phase = "model".into());
    let mut model_have_before: u64 = 0;
    for (i, f) in model.files.iter().enumerate() {
        let path = paths.model_file(model, f);
        // Bytes of the files before this one, done or not, so the bar never
        // steps back when a later file resumes from a part.
        let others_after: u64 = model_status.files[i + 1..].iter().map(|x| x.have).sum();
        if model_status.files[i].ready {
            model_have_before += f.size;
            continue;
        }
        setup.update(|s| s.file = f.name.into());
        let before = model_have_before;
        let mut report = |p: Progress| {
            setup.update(|s| s.done = base + before + p.done + others_after);
            !setup.stop.load(Ordering::SeqCst)
        };
        download::fetch(&client, &model.url(f), &path, f.size, f.sha256, &mut report)
            .with_context(|| format!("fetching {}/{}", model.id, f.name))?;
        model_have_before += f.size;
    }
    Ok(())
}

/// Removes one model's files (and parts) from disk, freeing its 2.5 GB; the
/// runtime stays. Returns the bytes removed.
pub fn remove_model(paths: &Paths, model: &ModelSpec) -> Result<u64> {
    let dir = paths.model_dir(model);
    let mut freed = 0u64;
    for f in model.files {
        let path = dir.join(f.name);
        for p in [path.clone(), partial(&path), sidecar(&path)] {
            if let Ok(meta) = fs::metadata(&p) {
                freed += meta.len();
                fs::remove_file(&p).with_context(|| format!("removing {}", p.display()))?;
            }
        }
    }
    let _ = fs::remove_dir(&dir);
    Ok(freed)
}

/// Removes the runtime library and every model: everything under
/// `<home>/voice` but the lock. Returns the bytes removed.
pub fn remove_all(paths: &Paths) -> Result<u64> {
    let mut freed = 0u64;
    for m in catalogue::MODELS {
        freed += remove_model(paths, m)?;
    }
    let runtime_root = paths.root.join("runtime");
    if runtime_root.exists() {
        freed += dir_size(&runtime_root);
        fs::remove_dir_all(&runtime_root)
            .with_context(|| format!("removing {}", runtime_root.display()))?;
    }
    let _ = fs::remove_dir(paths.root.join("models"));
    Ok(freed)
}

fn dir_size(dir: &std::path::Path) -> u64 {
    let mut n = 0;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                n += dir_size(&p);
            } else if let Ok(m) = fs::metadata(&p) {
                n += m.len();
            }
        }
    }
    n
}

/// `2.5 GB`, `36.9 MB`, `980 B`: the words the console and the command use.
pub fn size_words(bytes: u64) -> String {
    const KB: f64 = 1000.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.1} GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.0} kB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark_ready(path: &std::path::Path, size: u64, sha: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0u8; size as usize]).unwrap();
        fs::write(sidecar(path), sha).unwrap();
    }

    #[test]
    fn status_counts_parts_and_checked_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_str().unwrap());
        let spec = catalogue::model_or_default("");
        let st = status(&paths, "", "", None, None);
        assert_eq!(st.model.id, catalogue::DEFAULT_MODEL);
        assert_eq!(st.language, "auto");
        assert!(!st.model.ready && !st.ready);
        assert_eq!(st.model.have, 0);
        assert_eq!(st.models.len(), catalogue::MODELS.len());
        assert_eq!(st.supported, catalogue::runtime().is_some());
        assert!(st.root.ends_with("voice"));

        // The small files checked, a part of a big one on the way.
        let cfg = &spec.files[0];
        mark_ready(&paths.model_file(spec, cfg), cfg.size, cfg.sha256);
        let data = spec
            .files
            .iter()
            .find(|f| f.name == "encoder.onnx.data")
            .unwrap();
        let part = partial(&paths.model_file(spec, data));
        fs::write(&part, vec![1u8; 1000]).unwrap();
        let st = status(&paths, spec.id, "de-DE", Some("x".into()), None);
        assert_eq!(st.model.have, cfg.size + 1000);
        assert_eq!(st.language, "de-DE");
        assert_eq!(st.loaded.as_deref(), Some("x"));
        let f = st
            .model
            .files
            .iter()
            .find(|f| f.name == "config.json")
            .unwrap();
        assert!(f.ready && f.have == cfg.size);
        let f = st
            .model
            .files
            .iter()
            .find(|f| f.name == "encoder.onnx.data")
            .unwrap();
        assert!(!f.ready && f.have == 1000);
        // A file of the right size without its sidecar is not ready.
        let tok = &spec.files[1];
        fs::write(paths.model_file(spec, tok), vec![0u8; tok.size as usize]).unwrap();
        let st = status(&paths, spec.id, "", None, None);
        let f = st
            .model
            .files
            .iter()
            .find(|f| f.name == "tokenizer.model")
            .unwrap();
        assert!(!f.ready && f.have == 0);
    }

    #[test]
    fn remove_model_frees_files_parts_and_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_str().unwrap());
        let spec = catalogue::model_or_default("");
        let cfg = &spec.files[0];
        mark_ready(&paths.model_file(spec, cfg), 10, "abc");
        fs::write(partial(&paths.model_file(spec, &spec.files[1])), b"12345").unwrap();
        let freed = remove_model(&paths, spec).unwrap();
        assert_eq!(freed, 10 + 3 + 5);
        assert!(!paths.model_dir(spec).exists());
        assert_eq!(remove_model(&paths, spec).unwrap(), 0);
        if let Some(rt) = catalogue::runtime() {
            mark_ready(&paths.runtime_library(rt), 7, "h");
            assert_eq!(remove_all(&paths).unwrap(), 8);
            assert!(!paths.root.join("runtime").exists());
        }
    }

    #[test]
    fn setup_refuses_a_second_run_and_a_taken_lock() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_str().unwrap());
        let spec = catalogue::model_or_default("");
        let setup = Setup::default();
        assert_eq!(setup.snapshot(), SetupSnapshot::default());
        setup.running.store(true, Ordering::SeqCst);
        let err = run(&paths, spec, &setup).unwrap_err().to_string();
        assert!(err.contains("already running"), "{err}");
        setup.running.store(false, Ordering::SeqCst);

        if catalogue::runtime().is_none() {
            let err = run(&paths, spec, &setup).unwrap_err().to_string();
            assert!(err.contains("no ONNX Runtime build"), "{err}");
            return;
        }
        fs::create_dir_all(&paths.root).unwrap();
        let held = fs::File::create(paths.lock_file()).unwrap();
        held.lock_exclusive().unwrap();
        let err = run(&paths, spec, &setup).unwrap_err().to_string();
        assert!(err.contains("another voice setup holds"), "{err}");
        let snap = setup.snapshot();
        assert_eq!(snap.phase, "failed");
        assert!(snap.error.contains("another voice setup holds"));
        assert_eq!(snap.model, spec.id);
        assert!(!snap.started_at.is_empty() && !snap.finished_at.is_empty());
        assert!(!setup.running());
    }

    #[test]
    fn sizes_read_like_the_console() {
        assert_eq!(size_words(980), "980 B");
        assert_eq!(size_words(36_936_435), "36.9 MB");
        assert_eq!(size_words(2_495_361_024), "2.5 GB");
        assert_eq!(size_words(251_056), "251 kB");
    }
}
