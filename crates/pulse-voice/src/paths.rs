//! Where the voice files live under PULSE_HOME and what is there already.
//!
//! ```text
//! <home>/voice/runtime/<version>/<library>        the ONNX Runtime library
//! <home>/voice/models/<model id>/<file>           one model's files
//! <file>.part                                     a download in flight
//! <file>.sha256                                   the hash checked once the download finished
//! <home>/voice/setup.lock                         one setup at a time per home
//! ```

use crate::catalogue::{FileSpec, ModelSpec, RuntimeSpec};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn new(home: &str) -> Self {
        Self {
            root: Path::new(home).join("voice"),
        }
    }

    pub fn runtime_dir(&self, runtime: &RuntimeSpec) -> PathBuf {
        self.root.join("runtime").join(runtime.version)
    }

    pub fn runtime_library(&self, runtime: &RuntimeSpec) -> PathBuf {
        self.runtime_dir(runtime).join(runtime.library)
    }

    pub fn model_dir(&self, model: &ModelSpec) -> PathBuf {
        self.root.join("models").join(model.id)
    }

    pub fn model_file(&self, model: &ModelSpec, file: &FileSpec) -> PathBuf {
        self.model_dir(model).join(file.name)
    }

    pub fn lock_file(&self) -> PathBuf {
        self.root.join("setup.lock")
    }
}

/// The sidecar that records a finished, hash-checked download.
pub fn sidecar(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".sha256");
    path.with_file_name(name)
}

/// The partial file a download writes before it is complete.
pub fn partial(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

/// Whether `path` is a finished download of `size` bytes whose hash was
/// checked and recorded as `sha256`. Nothing is hashed here: a status read
/// must not spend seconds on 2.5 GB, so the check done once after the
/// download is what counts.
pub fn verified(path: &Path, size: u64, sha256: &str) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() != size {
        return false;
    }
    std::fs::read_to_string(sidecar(path))
        .map(|s| s.trim() == sha256)
        .unwrap_or(false)
}

/// Bytes a partial download already holds (0 when there is none).
pub fn partial_len(path: &Path) -> u64 {
    std::fs::metadata(partial(path))
        .map(|m| m.len())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{MODELS, RUNTIMES};

    #[test]
    fn files_sit_under_home_voice() {
        let p = Paths::new("/tmp/h");
        let rt = &RUNTIMES[0];
        assert_eq!(
            p.runtime_library(rt),
            PathBuf::from("/tmp/h/voice/runtime/1.31.1").join(rt.library)
        );
        let m = &MODELS[0];
        assert_eq!(
            p.model_file(m, &m.files[0]),
            PathBuf::from("/tmp/h/voice/models")
                .join(m.id)
                .join("config.json")
        );
        assert_eq!(p.lock_file(), PathBuf::from("/tmp/h/voice/setup.lock"));
        assert_eq!(
            sidecar(Path::new("/x/encoder.onnx.data")),
            PathBuf::from("/x/encoder.onnx.data.sha256")
        );
        assert_eq!(
            partial(Path::new("/x/encoder.onnx")),
            PathBuf::from("/x/encoder.onnx.part")
        );
    }

    #[test]
    fn verified_needs_size_and_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.bin");
        std::fs::write(&f, b"hello").unwrap();
        assert!(!verified(&f, 5, "abc"), "no sidecar yet");
        std::fs::write(sidecar(&f), "abc\n").unwrap();
        assert!(verified(&f, 5, "abc"));
        assert!(!verified(&f, 6, "abc"), "size differs");
        assert!(!verified(&f, 5, "abd"), "hash differs");
        assert_eq!(partial_len(&f), 0);
        std::fs::write(partial(&f), b"he").unwrap();
        assert_eq!(partial_len(&f), 2);
    }
}
