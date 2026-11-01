//! What the microphone needs on this machine and where it comes from: the
//! ONNX Runtime library for this platform and the speech models, every file
//! with its size and SHA-256 so a download is checked before it is used.
//!
//! The models are NVIDIA's Nemotron streaming ASR exports, the same ones the
//! GitHub Copilot CLI's `/voice` downloads: the multilingual 3.5 (default)
//! and the English-only one. They are the ONNX exports kept in the
//! `altunenes/parakeet-rs` Hugging Face repository, pinned to one revision.

/// One file of a model: its name inside the model folder, its size and hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSpec {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// A speech model the owner may choose (`voice.model`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSpec {
    /// The id the setting takes; also the folder name under `voice/models`.
    pub id: &'static str,
    /// Words for the console.
    pub title: &'static str,
    /// What it understands, in a few words.
    pub languages: &'static str,
    /// Hugging Face repository and the commit the hashes below belong to.
    pub repo: &'static str,
    pub revision: &'static str,
    /// Folder inside the repository.
    pub folder: &'static str,
    pub files: &'static [FileSpec],
}

impl ModelSpec {
    /// Bytes of every file together.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Where one file is fetched from.
    pub fn url(&self, file: &FileSpec) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}/{}",
            self.repo, self.revision, self.folder, file.name
        )
    }
}

/// The ONNX Runtime build for one platform: the release archive and the one
/// shared library taken out of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeSpec {
    pub version: &'static str,
    /// The platform word in the archive name (`osx-arm64`, `linux-x64`).
    pub platform: &'static str,
    pub archive_size: u64,
    pub archive_sha256: &'static str,
    /// The library's path inside the archive.
    pub member: &'static str,
    /// The library's file name on disk, its size and hash.
    pub library: &'static str,
    pub library_size: u64,
    pub library_sha256: &'static str,
}

impl RuntimeSpec {
    pub fn url(&self) -> String {
        format!(
            "https://github.com/microsoft/onnxruntime/releases/download/v{v}/onnxruntime-{p}-{v}.tgz",
            v = self.version,
            p = self.platform
        )
    }
}

pub const RUNTIME_VERSION: &str = "1.31.1";

/// The runtime builds this release knows. There is no build for Intel Macs
/// (the Copilot CLI leaves them out too) nor for Windows, which PULSE does not
/// run on.
pub const RUNTIMES: &[RuntimeSpec] = &[
    RuntimeSpec {
        version: RUNTIME_VERSION,
        platform: "osx-arm64",
        archive_size: 36_936_435,
        archive_sha256: "2e1344a1926ee50eb25b38153cb6992c3567515aef0e677a1a520a1798896d6f",
        member: "onnxruntime-osx-arm64-1.31.1/lib/libonnxruntime.1.31.1.dylib",
        library: "libonnxruntime.1.31.1.dylib",
        library_size: 44_797_256,
        library_sha256: "17500d6b1e37e70f43a2c57d94233a1bc29fbcb297be0a5f8c7c13184f3307ba",
    },
    RuntimeSpec {
        version: RUNTIME_VERSION,
        platform: "linux-x64",
        archive_size: 11_413_655,
        archive_sha256: "c849e36a1ef3e22d4ff217b9d1ce4a3cf5b9467b975610913ae959ad5b16bdc9",
        member: "onnxruntime-linux-x64-1.31.1/lib/libonnxruntime.so.1.31.1",
        library: "libonnxruntime.so.1.31.1",
        library_size: 29_345_384,
        library_sha256: "a96f132e5eb5ca04e23a7d2fc1078fb95aa591e8d71d0f237702ece3a1e0f4ea",
    },
    RuntimeSpec {
        version: RUNTIME_VERSION,
        platform: "linux-aarch64",
        archive_size: 10_332_381,
        archive_sha256: "5317505e2da99be65f8e2775ba3c2afd6d2961fd78aba1bd8f56159f48820c78",
        member: "onnxruntime-linux-aarch64-1.31.1/lib/libonnxruntime.so.1.31.1",
        library: "libonnxruntime.so.1.31.1",
        library_size: 25_266_408,
        library_sha256: "7e17c55d675be73f67a218dbecb55522395f61dafb3da03ebc7ade56a4c2f813",
    },
];

/// The platform word for this build of pulse, or `None` when no runtime is
/// in the catalogue for it.
pub fn platform() -> Option<&'static str> {
    platform_for(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn platform_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("osx-arm64"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        _ => None,
    }
}

/// The runtime build for this machine.
pub fn runtime() -> Option<&'static RuntimeSpec> {
    let p = platform()?;
    RUNTIMES.iter().find(|r| r.platform == p)
}

const PARAKEET_REPO: &str = "altunenes/parakeet-rs";
const PARAKEET_REVISION: &str = "664cc742197053fa11afd12f5100749fb8000181";

/// The model `voice.model` names when the owner never chose one.
pub const DEFAULT_MODEL: &str = "nemotron-3.5-asr-streaming-0.6b";

pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "nemotron-3.5-asr-streaming-0.6b",
        title: "Nemotron 3.5 ASR streaming 0.6B (multilingual)",
        languages: "25+ languages, picks the language itself or takes voice.language",
        repo: PARAKEET_REPO,
        revision: PARAKEET_REVISION,
        folder: "nemotron-3.5-asr-streaming-0.6b-onnx",
        files: &[
            FileSpec {
                name: "config.json",
                size: 2_995,
                sha256: "d538922d70151e2a0bc1153d8e02cdf341d8d059d1e33ab4365cdb1a958c3fcc",
            },
            FileSpec {
                name: "tokenizer.model",
                size: 406_554,
                sha256: "ce3895e40806f02a26c3a225161b96ef682d6c0054bae32a245dec4258d7d291",
            },
            FileSpec {
                name: "encoder.onnx",
                size: 4_161_952,
                sha256: "487519292b5b12178442dabc9ad5803f859354680ec929140d4027dceb16a20d",
            },
            FileSpec {
                name: "decoder_joint.onnx",
                size: 97_653_235,
                sha256: "a4258dfb3a4c5c15ef3ff7e52483bf9b4fcf721ae5a8d2dac777178e8fa39252",
            },
            FileSpec {
                name: "encoder.onnx.data",
                size: 2_495_361_024,
                sha256: "1cdddec47b0570170183716dff74df8ca827facb41b11302ebdd479616171988",
            },
        ],
    },
    ModelSpec {
        id: "nemotron-speech-streaming-en-0.6b",
        title: "Nemotron Speech streaming 0.6B (English)",
        languages: "English only",
        repo: PARAKEET_REPO,
        revision: PARAKEET_REVISION,
        folder: "nemotron-speech-streaming-en-0.6b",
        files: &[
            FileSpec {
                name: "config.json",
                size: 980,
                sha256: "c0e8b98c0f25182930e69ef35dbd71452d44fc2113d36d0bfe5bfa272fa6c950",
            },
            FileSpec {
                name: "tokenizer.model",
                size: 251_056,
                sha256: "07d4e5a63840a53ab2d4d106d2874768143fb3fbdd47938b3910d2da05bfb0a9",
            },
            FileSpec {
                name: "encoder.onnx",
                size: 4_026_060,
                sha256: "ac093bfc3f8d01355bc0860a94bed02a2f4db2bc1417c76b45c3b507d82f4d28",
            },
            FileSpec {
                name: "decoder_joint.onnx",
                size: 35_842_420,
                sha256: "637bac59ac84100896c30cee6b5d9493977460abcc997f3ce52c3122ff4b1bd5",
            },
            FileSpec {
                name: "encoder.onnx.data",
                size: 2_477_522_944,
                sha256: "992b42cb1deaca0b4758ab00dbb87f05065a32150c001c898f40287134387bac",
            },
        ],
    },
];

/// The model with this id.
pub fn model(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id.trim())
}

/// The model `voice.model` names, the default when the value is empty or
/// unknown (a store from before this model list, or a typo in a script).
pub fn model_or_default(id: &str) -> &'static ModelSpec {
    model(id)
        .or_else(|| model(DEFAULT_MODEL))
        .expect("the default model is in the catalogue")
}

/// The language words `voice.language` takes with the multilingual model:
/// `auto` lets the model pick; a locale tag fixes it. The English model
/// ignores the setting.
pub const LANGUAGES: &[&str] = &[
    "auto", "af-ZA", "am-ET", "ar-AR", "ay-BO", "az-AZ", "bg-BG", "bn-IN", "cs-CZ", "da-DK",
    "de-DE", "el-GR", "en-US", "en-GB", "es-ES", "es-US", "et-EE", "fa-IR", "fi-FI", "fr-CA",
    "fr-FR", "gn-PY", "gu-IN", "ha-NG", "haw-US", "he-IL", "hi-IN", "hr-HR", "hu-HU", "hy-AM",
    "id-ID", "ig-NG", "it-IT", "ja-JP", "ka-GE", "km-KH", "kn-IN", "ko-KR", "ku-TR", "ky-KG",
    "ln-CD", "lt-LT", "lv-LV", "mi-NZ", "ml-IN", "mr-IN", "ms-MY", "mt-MT", "nah-MX", "nb-NO",
    "ne-NP", "nl-NL", "nn-NO", "no-NO", "ny-MW", "or-KE", "pl-PL", "pt-BR", "pt-PT", "qu-PE",
    "ro-RO", "ru-RU", "rw-RW", "si-LK", "sk-SK", "sl-SI", "sm-WS", "so-SO", "sv-SE", "sw-KE",
    "ta-IN", "te-IN", "tg-TJ", "th-TH", "to-TO", "tr-TR", "uk-UA", "ur-PK", "uz-UZ", "vi-VN",
    "yo-NG", "zh-CN", "zh-TW", "zu-ZA",
];

/// Whether `voice.language` names a language the multilingual model takes:
/// a tag from [`LANGUAGES`] or its bare language part (`de` for `de-DE`).
pub fn known_language(lang: &str) -> bool {
    language_tag(lang).is_some()
}

/// The tag the model is told for `lang`: the tag itself when it is in
/// [`LANGUAGES`], else the first tag of that language (`de` → `de-DE`);
/// `None` for a word the model has no prompt for.
pub fn language_tag(lang: &str) -> Option<&'static str> {
    let lang = lang.trim();
    if lang.is_empty() {
        return None;
    }
    LANGUAGES
        .iter()
        .find(|l| **l == lang)
        .or_else(|| LANGUAGES.iter().find(|l| l.split('-').next() == Some(lang)))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hash_is_a_sha256_and_sizes_add_up() {
        let hex64 = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit());
        for r in RUNTIMES {
            assert!(hex64(r.archive_sha256), "{}", r.platform);
            assert!(hex64(r.library_sha256), "{}", r.platform);
            assert!(r.member.ends_with(r.library), "{}", r.platform);
            assert!(r.url().contains(r.platform));
            assert!(r.url().contains(RUNTIME_VERSION));
        }
        for m in MODELS {
            assert!(m.size() > 2_000_000_000, "{} is a 0.6B fp32 model", m.id);
            for f in m.files {
                assert!(hex64(f.sha256), "{}/{}", m.id, f.name);
                assert!(m.url(f).contains(&format!("{}/{}", m.folder, f.name)));
            }
            let names: Vec<_> = m.files.iter().map(|f| f.name).collect();
            for need in [
                "encoder.onnx",
                "encoder.onnx.data",
                "decoder_joint.onnx",
                "tokenizer.model",
            ] {
                assert!(names.contains(&need), "{} lacks {need}", m.id);
            }
        }
        assert_eq!(model_or_default("").id, DEFAULT_MODEL);
        assert_eq!(model_or_default("bogus").id, DEFAULT_MODEL);
        assert_eq!(
            model_or_default(" nemotron-speech-streaming-en-0.6b ").languages,
            "English only"
        );
    }

    #[test]
    fn platforms_follow_the_release_assets() {
        assert_eq!(platform_for("macos", "aarch64"), Some("osx-arm64"));
        assert_eq!(platform_for("linux", "x86_64"), Some("linux-x64"));
        assert_eq!(platform_for("linux", "aarch64"), Some("linux-aarch64"));
        assert_eq!(platform_for("macos", "x86_64"), None);
        assert_eq!(platform_for("windows", "x86_64"), None);
        for p in ["osx-arm64", "linux-x64", "linux-aarch64"] {
            assert!(RUNTIMES.iter().any(|r| r.platform == p));
        }
    }

    #[test]
    fn languages_are_distinct_and_auto_leads() {
        assert_eq!(LANGUAGES[0], "auto");
        let mut sorted = LANGUAGES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), LANGUAGES.len());
        assert!(known_language("auto"));
        assert!(known_language("de-DE"));
        assert_eq!(language_tag("de"), Some("de-DE"));
        assert_eq!(
            language_tag(" en "),
            Some("en-US"),
            "the first en tag in the list"
        );
        assert_eq!(language_tag("en-US"), Some("en-US"));
        assert!(!known_language("klingon"));
        assert!(!known_language(""));
    }
}
