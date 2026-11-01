//! The microphone's routes: what speech model is on this machine, the setup
//! that fetches it, and the transcription itself. Audio comes from the
//! console as raw 16 kHz 16-bit PCM and the text goes back; nothing leaves
//! the machine (ADR: the model runs in this process through ONNX Runtime).
//!
//! `GET  /api/v1/voice`                 the status the composer's button shows
//! `POST /api/v1/voice/setup`           `{model?}` starts the download; `{stop:true}` stops it
//! `POST /api/v1/voice/transcribe`      body `audio/pcm` → `{text, ...}`
//! `POST /api/v1/voice/unload`          drops the model from memory
//! `DELETE /api/v1/voice/model?id=…`    removes a model's files (`all=1` for everything)

use crate::additions::{addition_failure, addition_method, addition_not_found};
use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, write_error, write_json};
use pulse_voice::{Engine, Paths, Setup};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One recognizer per hub, shared by every request: the model in memory,
/// the setup in flight and the thread that drops the model when idle.
#[derive(Clone, Default)]
pub(crate) struct Voice {
    pub(crate) engine: Engine,
    pub(crate) setup: Setup,
    /// Set once the idle-release thread runs (one per hub).
    idle_watch: Arc<Mutex<bool>>,
}

/// How long the model stays in memory after the last transcription. A
/// 0.6B model holds about 2.5 GB; a minute of silence in the conversation
/// is long enough that the next press can afford the ~2 s reload.
pub(crate) const IDLE_RELEASE: Duration = Duration::from_secs(10 * 60);

/// The longest recording the route takes, as bytes of 16 kHz 16-bit mono.
const PCM_LIMIT: usize = pulse_voice::audio::MAX_PCM_BYTES;

impl Voice {
    /// Starts the thread that frees the model after [`IDLE_RELEASE`] of
    /// silence, once.
    fn watch_idle(&self) {
        let mut started = self.idle_watch.lock().unwrap_or_else(|e| e.into_inner());
        if *started {
            return;
        }
        *started = true;
        let engine = self.engine.clone();
        std::thread::Builder::new()
            .name("voice-idle".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(30));
                    engine.release_idle(IDLE_RELEASE);
                }
            })
            .ok();
    }
}

impl Handler {
    pub(crate) fn addition_voice(&self, w: &mut ResponseWriter, r: &Request, parts: Vec<String>) {
        match parts.as_slice() {
            [_] => {
                if addition_method(w, r, vec!["GET".to_string()]) {
                    self.voice_status(w);
                }
            }
            [_, action] if action == "setup" => {
                if addition_method(w, r, vec!["POST".to_string()]) {
                    self.voice_setup(w, r);
                }
            }
            [_, action] if action == "transcribe" => {
                if addition_method(w, r, vec!["POST".to_string()]) {
                    self.voice_transcribe(w, r);
                }
            }
            [_, action] if action == "unload" => {
                if addition_method(w, r, vec!["POST".to_string()])
                    && crate::server::same_origin_write(w, r)
                {
                    self.voice.engine.unload();
                    self.voice_status(w);
                }
            }
            [_, action] if action == "model" => {
                if addition_method(w, r, vec!["DELETE".to_string()]) {
                    self.voice_remove(w, r);
                }
            }
            _ => addition_not_found(w),
        }
    }

    fn voice_paths(&self, w: &mut ResponseWriter) -> Option<(Paths, String, String)> {
        let Some(store) = self.store.as_deref() else {
            write_error(
                w,
                503,
                "The store is unavailable".to_string(),
                "The hub has no open PULSE store.".to_string(),
                "Run pulse init, then restart pulse hub.".to_string(),
            );
            return None;
        };
        let model = store.setting(pulse_voice::MODEL_SETTING, "");
        let language = store.setting(pulse_voice::LANGUAGE_SETTING, "");
        Some((Paths::new(&store.home), model, language))
    }

    fn voice_status_json(&self, paths: &Paths, model: &str, language: &str) -> serde_json::Value {
        let setup = self.voice.setup.snapshot();
        let setup = if setup.phase.is_empty() {
            None
        } else {
            Some(setup)
        };
        let st =
            pulse_voice::setup::status(paths, model, language, self.voice.engine.loaded(), setup);
        let mut v = serde_json::to_value(&st).unwrap_or_else(|_| json!({}));
        if let Some(o) = v.as_object_mut() {
            o.insert("setting_up".into(), json!(self.voice.setup.running()));
            o.insert(
                "idle_release_minutes".into(),
                json!(IDLE_RELEASE.as_secs() / 60),
            );
            o.insert("languages".into(), json!(pulse_voice::catalogue::LANGUAGES));
            o.insert("max_seconds".into(), json!(pulse_voice::audio::MAX_SECONDS));
        }
        v
    }

    fn voice_status(&self, w: &mut ResponseWriter) {
        let Some((paths, model, language)) = self.voice_paths(w) else {
            return;
        };
        let v = self.voice_status_json(&paths, &model, &language);
        write_json(w, 200, v);
    }

    /// `POST /voice/setup` with `{model?: ID}` starts fetching the runtime
    /// and the model in a thread and answers 202 with the status; the
    /// console polls `GET /voice`. `{stop: true}` asks the running setup
    /// to stop after its current chunk (parts stay for a resume).
    fn voice_setup(&self, w: &mut ResponseWriter, r: &Request) {
        let (payload, ok) = crate::server::read_payload(w, r);
        if !ok {
            return;
        }
        let Some((paths, setting_model, language)) = self.voice_paths(w) else {
            return;
        };
        if payload.get("stop").and_then(|v| v.as_bool()) == Some(true) {
            self.voice.setup.stop();
            let v = self.voice_status_json(&paths, &setting_model, &language);
            write_json(w, 202, v);
            return;
        }
        let model = payload
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| setting_model.clone());
        let Some(spec) = pulse_voice::catalogue::model(&model) else {
            write_error(
                w,
                400,
                format!("Unknown model {model:?}"),
                format!(
                    "The models are {}.",
                    pulse_voice::catalogue::MODELS
                        .iter()
                        .map(|m| m.id)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                "Pick one of them, or leave model out for the voice.model setting.".to_string(),
            );
            return;
        };
        if pulse_voice::catalogue::runtime().is_none() {
            write_error(
                w,
                409,
                "No speech runtime for this machine".to_string(),
                format!(
                    "There is no ONNX Runtime build for {} {}.",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
                "The microphone needs an Apple-silicon Mac or an x86-64/arm64 Linux machine."
                    .to_string(),
            );
            return;
        }
        if self.voice.setup.running() {
            write_error(
                w,
                409,
                "A setup is already running".to_string(),
                format!(
                    "This hub is fetching {}.",
                    self.voice.setup.snapshot().model
                ),
                "Wait for it, or send {\"stop\": true} first.".to_string(),
            );
            return;
        }
        // The chosen model is also what the microphone will use: a setup
        // for the other model switches the setting so the button, the
        // status and the files agree.
        if model != setting_model
            && let Some(store) = self.store.as_deref()
            && let Err(e) = store.set_setting("hub", pulse_voice::MODEL_SETTING, &model)
        {
            addition_failure(w, e);
            return;
        }
        let setup = self.voice.setup.clone();
        let thread_paths = paths.clone();
        let spawned = std::thread::Builder::new()
            .name("voice-setup".into())
            .spawn(move || {
                let _ = pulse_voice::setup::run(&thread_paths, spec, &setup);
            });
        if let Err(e) = spawned {
            addition_failure(w, format!("starting the setup thread: {e}"));
            return;
        }
        // Give the thread a moment to take the lock and report its phase,
        // so the answer already shows the setup running.
        for _ in 0..20 {
            if self.voice.setup.running() || !self.voice.setup.snapshot().phase.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let v = self.voice_status_json(&paths, &model, &language);
        write_json(w, 202, v);
    }

    /// `POST /voice/transcribe` with the recording as the body: raw
    /// little-endian 16-bit mono PCM at 16 kHz (`Content-Type: audio/pcm`,
    /// the console's format) or a WAV file (`audio/wav`). `language=` in
    /// the query overrides `voice.language` for this request.
    fn voice_transcribe(&self, w: &mut ResponseWriter, r: &Request) {
        if !crate::server::same_origin_write(w, r) {
            return;
        }
        let Some((paths, model, setting_language)) = self.voice_paths(w) else {
            return;
        };
        let media = r
            .header("Content-Type")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if r.body.is_empty() {
            write_error(
                w,
                400,
                "No audio".to_string(),
                "The request body is empty.".to_string(),
                "Send the recording as the body: audio/pcm (16 kHz, 16-bit, mono) or audio/wav."
                    .to_string(),
            );
            return;
        }
        if r.body.len() > PCM_LIMIT + 1024 {
            write_error(
                w,
                413,
                "Recording too long".to_string(),
                format!(
                    "The body is {} bytes; the microphone takes up to {} seconds ({} bytes of 16 kHz PCM).",
                    r.body.len(),
                    pulse_voice::audio::MAX_SECONDS,
                    PCM_LIMIT
                ),
                "Stop the recording sooner and send it in parts.".to_string(),
            );
            return;
        }
        let samples = match media.as_str() {
            "audio/wav" | "audio/x-wav" | "audio/wave" => {
                pulse_voice::audio::wav_to_samples(&r.body).map(|(s, _)| s)
            }
            "audio/pcm" | "audio/l16" | "application/octet-stream" | "" => {
                pulse_voice::audio::pcm16_to_samples(&r.body)
            }
            other => {
                write_error(
                    w,
                    415,
                    format!("Content-Type {other} is not audio the hub reads"),
                    "The microphone route takes audio/pcm (16 kHz, 16-bit, mono) or audio/wav."
                        .to_string(),
                    "Send Content-Type: audio/pcm.".to_string(),
                );
                return;
            }
        };
        let samples = match samples {
            Ok(s) => s,
            Err(e) => {
                write_error(
                    w,
                    400,
                    "Unreadable audio".to_string(),
                    e.to_string(),
                    "Send 16 kHz 16-bit mono PCM, or a WAV file.".to_string(),
                );
                return;
            }
        };
        let spec = pulse_voice::catalogue::model_or_default(&model);
        let st = pulse_voice::setup::status(&paths, &model, &setting_language, None, None);
        if !st.ready {
            write_error(
                w,
                409,
                "The speech model is not set up".to_string(),
                if st.supported {
                    format!(
                        "{} is {} of {} on this machine.",
                        spec.id,
                        pulse_voice::setup::size_words(st.model.have),
                        pulse_voice::setup::size_words(st.model.size)
                    )
                } else {
                    format!(
                        "There is no ONNX Runtime build for {} {}.",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    )
                },
                if st.supported {
                    "Start the setup from the microphone button, or run `pulse voice setup`."
                        .to_string()
                } else {
                    "The microphone needs an Apple-silicon Mac or an x86-64/arm64 Linux machine."
                        .to_string()
                },
            );
            return;
        }
        let query_language = r.query("language");
        let language = if query_language.trim().is_empty() {
            setting_language
        } else {
            query_language
        };
        self.voice.watch_idle();
        match self.voice.engine.transcribe(&paths, spec, &language, &samples) {
            Ok(t) => write_json(w, 200, json!({"transcript": t})),
            Err(e) => write_error(
                w,
                500,
                "Transcription failed".to_string(),
                format!("{e:#}"),
                "Try again; if it keeps failing, run `pulse voice remove` and set the model up again.".to_string(),
            ),
        }
    }

    /// `DELETE /voice/model?id=MODEL` (or `?all=1`) removes files from disk;
    /// the model in memory is dropped first.
    fn voice_remove(&self, w: &mut ResponseWriter, r: &Request) {
        if !crate::server::same_origin_write(w, r) {
            return;
        }
        let Some((paths, setting_model, language)) = self.voice_paths(w) else {
            return;
        };
        if self.voice.setup.running() {
            write_error(
                w,
                409,
                "A setup is running".to_string(),
                "The files are being written right now.".to_string(),
                "Stop the setup first.".to_string(),
            );
            return;
        }
        self.voice.engine.unload();
        let all = matches!(r.query("all").as_str(), "1" | "true" | "yes");
        let id = r.query("id");
        let result = if all {
            pulse_voice::setup::remove_all(&paths)
        } else {
            let id = if id.trim().is_empty() {
                setting_model.clone()
            } else {
                id.trim().to_string()
            };
            match pulse_voice::catalogue::model(&id) {
                Some(spec) => pulse_voice::setup::remove_model(&paths, spec),
                None => {
                    write_error(
                        w,
                        400,
                        format!("Unknown model {id:?}"),
                        "No such model is in the catalogue.".to_string(),
                        "Name a model from GET /api/v1/voice, or pass all=1.".to_string(),
                    );
                    return;
                }
            }
        };
        match result {
            Ok(freed) => {
                let mut v = self.voice_status_json(&paths, &setting_model, &language);
                if let Some(o) = v.as_object_mut() {
                    o.insert("freed".into(), json!(freed));
                }
                write_json(w, 200, v);
            }
            Err(e) => addition_failure(w, e),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_helpers_http::{additions_store, handler, request};

    #[test]
    fn voice_status_names_the_models_and_what_is_missing() {
        let _serial = crate::test_helpers::serial();
        let s = additions_store(true);
        let h = handler(&s);
        let (status, result, _) = request(h.clone_for_test(), "GET", "/api/v1/voice", "", &[]);
        assert_eq!(status, 200, "{result:#?}");
        assert_eq!(result["model"]["id"], "nemotron-3.5-asr-streaming-0.6b");
        assert_eq!(result["model"]["ready"], false);
        assert_eq!(result["ready"], false);
        assert_eq!(result["language"], "auto");
        assert_eq!(result["setting_up"], false);
        assert!(result["setup"].is_null(), "{result:#?}");
        assert_eq!(result["models"].as_array().map(Vec::len), Some(2));
        assert_eq!(result["languages"][0], "auto");
        let languages = result["languages"].as_array().unwrap();
        assert!(languages.iter().any(|l| l == "en-US"), "{languages:?}");
        assert_eq!(result["max_seconds"], 300);
        assert!(
            result["root"].as_str().unwrap().ends_with("/voice"),
            "{result:#?}"
        );
        assert!(result["loaded"].is_null());

        // The other model's status is in the list too, with its files.
        let en = &result["models"][1];
        assert_eq!(en["id"], "nemotron-speech-streaming-en-0.6b");
        assert_eq!(en["files"].as_array().map(Vec::len), Some(5));
        assert_eq!(en["have"], 0);

        // Unknown sub-routes and wrong methods are refused as elsewhere.
        let (status, _, _) = request(h.clone_for_test(), "GET", "/api/v1/voice/nope", "", &[]);
        assert_eq!(status, 404);
        let (status, _, _) = request(h.clone_for_test(), "POST", "/api/v1/voice", "{}", &[]);
        assert_eq!(status, 405);
        let (status, _, _) = request(h.clone_for_test(), "GET", "/api/v1/voice/setup", "", &[]);
        assert_eq!(status, 405);
    }

    #[test]
    fn transcribe_refuses_bad_audio_and_a_model_that_is_not_set_up() {
        let _serial = crate::test_helpers::serial();
        let s = additions_store(true);
        let h = handler(&s);
        let srv = crate::test_helpers_http::server(h.clone_for_test());
        let client = crate::test_helpers::client();
        let url = format!("{}/api/v1/voice/transcribe", srv.url);

        // Nothing in the body.
        let resp = client
            .post(&url)
            .header("Content-Type", "audio/pcm")
            .body(Vec::<u8>::new())
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 400);

        // A media type the route does not read.
        let resp = client
            .post(&url)
            .header("Content-Type", "text/plain")
            .body("hello")
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 415);

        // Odd byte count is not 16-bit PCM.
        let resp = client
            .post(&url)
            .header("Content-Type", "audio/pcm")
            .body(vec![0u8; 3])
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 400);
        let body: serde_json::Value = resp.json().unwrap();
        assert_eq!(body["error"], "Unreadable audio");

        // Good PCM, but no model on disk: a 409 that says how to set it up.
        let resp = client
            .post(&url)
            .header("Content-Type", "audio/pcm")
            .body(vec![0u8; 32_000])
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 409);
        let body: serde_json::Value = resp.json().unwrap();
        assert_eq!(body["error"], "The speech model is not set up");
        assert!(
            body["fix"].as_str().unwrap().contains("pulse voice setup"),
            "{body:#?}"
        );

        // Cross-origin writes are refused before anything is read.
        let resp = client
            .post(&url)
            .header("Content-Type", "audio/pcm")
            .header("Origin", "http://evil.example")
            .body(vec![0u8; 32_000])
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 403);
    }

    #[test]
    fn setup_validates_the_model_and_remove_reports_what_it_freed() {
        let _serial = crate::test_helpers::serial();
        let s = additions_store(true);
        let h = handler(&s);
        let (status, result, _) = request(
            h.clone_for_test(),
            "POST",
            "/api/v1/voice/setup",
            r#"{"model":"whisper-large"}"#,
            &[],
        );
        assert_eq!(status, 400, "{result:#?}");
        assert!(
            result["why"]
                .as_str()
                .unwrap()
                .contains("nemotron-3.5-asr-streaming-0.6b"),
            "{result:#?}"
        );

        // Stop with nothing running is harmless and answers the status.
        let (status, result, _) = request(
            h.clone_for_test(),
            "POST",
            "/api/v1/voice/setup",
            r#"{"stop":true}"#,
            &[],
        );
        assert_eq!(status, 202, "{result:#?}");
        assert_eq!(result["setting_up"], false);

        // Removing a model that was never fetched frees nothing; an unknown
        // id is refused.
        let (status, result, _) = request(
            h.clone_for_test(),
            "DELETE",
            "/api/v1/voice/model?id=nemotron-speech-streaming-en-0.6b",
            "",
            &[],
        );
        assert_eq!(status, 200, "{result:#?}");
        assert_eq!(result["freed"], 0);
        let (status, _, _) = request(
            h.clone_for_test(),
            "DELETE",
            "/api/v1/voice/model?id=bogus",
            "",
            &[],
        );
        assert_eq!(status, 400);

        // Unload with nothing loaded answers the status too.
        let (status, result, _) = request(
            h.clone_for_test(),
            "POST",
            "/api/v1/voice/unload",
            "{}",
            &[],
        );
        assert_eq!(status, 200, "{result:#?}");
        assert!(result["loaded"].is_null());
    }
}
