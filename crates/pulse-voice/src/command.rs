//! `pulse voice`: the microphone's files from the command line - what is
//! there, fetch what is missing, try a WAV, free the disk.

use crate::catalogue;
use crate::engine::Engine;
use crate::paths::Paths;
use crate::setup::{self, Setup, size_words};
use anyhow::Result;
use pulse_cli::{Command, Context, FlagSet, errorf, register as cli_register, write_json};
use std::io::Write;
use std::sync::Once;

static REGISTER: Once = Once::new();

pub const MODEL_SETTING: &str = "voice.model";
pub const LANGUAGE_SETTING: &str = "voice.language";

pub fn register() {
    REGISTER.call_once(|| {
        cli_register(Command {
            name: "voice".into(),
            group: "human".into(),
            summary: "speech to text for the hub's microphone: set up the model on this machine, try it on a WAV, remove it".into(),
            usage: "pulse voice [status] [--json]\n       pulse voice setup [--model ID] [--json]          fetch ONNX Runtime and the model voice.model names (or --model) into PULSE_HOME/voice\n       pulse voice transcribe FILE.wav [--language TAG] [--json]\n       pulse voice remove [--model ID|--all] [--json]   free the disk; the setting stays".into(),
            aliases: vec![],
            no_store: false,
            read_only: true,
            raw: false,
            run,
            flags: Some(flags),
        });
    });
}

fn flags(fs: &mut FlagSet) {
    fs.string_var(
        "model",
        "",
        "a model id from `pulse voice status` (default: the voice.model setting)",
    );
    fs.string_var(
        "language",
        "",
        "a language tag for the multilingual model (default: the voice.language setting, auto)",
    );
    fs.bool_var("all", false, "with remove: the runtime and every model");
}

fn run(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    let sub = args.first().map(String::as_str).unwrap_or("status");
    let ok_shape = match sub {
        "status" | "setup" | "remove" => args.len() <= 1,
        "transcribe" => args.len() == 2,
        _ => false,
    };
    if !ok_shape {
        return Err(errorf(
            2,
            "Invalid voice command",
            "Use status, setup [--model ID], transcribe FILE.wav [--language TAG], or remove [--model ID|--all].",
            "pulse help voice",
        )
        .into());
    }
    let home = c
        .store
        .as_ref()
        .map(|s| s.home.clone())
        .unwrap_or_else(|| c.home.clone());
    let paths = Paths::new(&home);
    let (setting_model, setting_language) = match c.store.as_ref() {
        Some(s) => (
            s.setting(MODEL_SETTING, ""),
            s.setting(LANGUAGE_SETTING, ""),
        ),
        None => (String::new(), String::new()),
    };
    let model_arg = fs.get_string("model");
    let model_id = if model_arg.trim().is_empty() {
        setting_model.clone()
    } else {
        model_arg.clone()
    };
    if !model_arg.trim().is_empty() && catalogue::model(&model_arg).is_none() {
        return Err(errorf(
            2,
            &format!("Unknown model {:?}", model_arg.trim()),
            &format!(
                "The models are: {}.",
                catalogue::MODELS
                    .iter()
                    .map(|m| m.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "pulse voice status",
        )
        .into());
    }
    let spec = catalogue::model_or_default(&model_id);
    let language = {
        let l = fs.get_string("language");
        if l.trim().is_empty() {
            setting_language
        } else {
            l
        }
    };

    match sub {
        "status" => {
            let st = setup::status(&paths, &model_id, &language, None, None);
            if c.json {
                return write_json(c.out.as_mut(), &st);
            }
            if c.quiet {
                return Ok(());
            }
            if !st.supported {
                writeln!(
                    c.out,
                    "voice: no ONNX Runtime build for {} {}; the microphone needs an Apple-silicon Mac or an x86-64/arm64 Linux machine",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )?;
            } else if let Some(rt) = &st.runtime {
                writeln!(
                    c.out,
                    "runtime  ONNX Runtime {} ({})  {}  {}",
                    rt.version,
                    rt.platform,
                    size_words(rt.size),
                    if rt.ready { "ready" } else { "not set up" }
                )?;
            }
            for m in &st.models {
                let chosen = if m.id == st.model.id { "*" } else { " " };
                let state = if m.ready {
                    "ready".to_string()
                } else if m.have > 0 {
                    format!("{} of {}", size_words(m.have), size_words(m.size))
                } else {
                    format!("not set up ({})", size_words(m.size))
                };
                writeln!(c.out, "{chosen} {:<36} {:<48} {state}", m.id, m.title)?;
            }
            writeln!(
                c.out,
                "language {}   files under {}   {} free",
                st.language,
                st.root,
                size_words(st.free_bytes)
            )?;
            if st.ready {
                writeln!(
                    c.out,
                    "ready: the hub's microphone transcribes with {}",
                    st.model.id
                )?;
            } else {
                writeln!(c.out, "run `pulse voice setup` to fetch what is missing")?;
            }
            Ok(())
        }
        "setup" => {
            let shared = Setup::default();
            let quiet = c.quiet || c.json;
            if !quiet {
                let st = setup::model_status(&paths, spec);
                writeln!(
                    c.out,
                    "setting up {} ({}, {} of {} on disk) under {}",
                    spec.id,
                    spec.languages,
                    size_words(st.have),
                    size_words(st.size),
                    paths.root.display()
                )?;
            }
            // Progress on stderr once a second while the download runs.
            let reporter = {
                let shared = shared.clone();
                let quiet = quiet || !c.color;
                std::thread::spawn(move || {
                    if quiet {
                        return;
                    }
                    let mut last = String::new();
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        let s = shared.snapshot();
                        if !shared.running() {
                            break;
                        }
                        let line = format!(
                            "{}: {} {} of {}",
                            s.phase,
                            s.file,
                            size_words(s.done),
                            size_words(s.total)
                        );
                        if line != last {
                            eprint!("\r\x1b[2K{line}");
                            last = line;
                        }
                    }
                    eprint!("\r\x1b[2K");
                })
            };
            let result = setup::run(&paths, spec, &shared);
            let _ = reporter.join();
            let snap = shared.snapshot();
            if c.json {
                write_json(c.out.as_mut(), &snap)?;
            }
            result?;
            if !quiet {
                writeln!(
                    c.out,
                    "done: {} ready; the hub's microphone transcribes on this machine",
                    spec.id
                )?;
            }
            Ok(())
        }
        "transcribe" => {
            let file = &args[1];
            let bytes = std::fs::read(file).map_err(|e| {
                errorf(
                    2,
                    &format!("Cannot read {file}"),
                    &e.to_string(),
                    "Name a WAV file (16-bit or float, any rate; mono or stereo).",
                )
            })?;
            let (samples, info) = crate::audio::wav_to_samples(&bytes).map_err(|e| {
                errorf(
                    2,
                    &format!("Cannot decode {file}"),
                    &e.to_string(),
                    "Name a WAV file (16-bit or float, any rate; mono or stereo).",
                )
            })?;
            let engine = Engine::default();
            let t = engine.transcribe(&paths, spec, &language, &samples)?;
            if c.json {
                return write_json(c.out.as_mut(), &t);
            }
            if c.quiet {
                return Ok(());
            }
            writeln!(c.out, "{}", t.text)?;
            writeln!(
                c.err,
                "{} ({} Hz, {} ch, {:.1}s) -> {} in {} ms{}",
                file,
                info.sample_rate,
                info.channels,
                info.seconds,
                t.model,
                t.took_ms,
                if t.language.is_empty() {
                    String::new()
                } else {
                    format!(", language {}", t.language)
                }
            )?;
            Ok(())
        }
        "remove" => {
            let freed = if fs.get_bool("all") {
                setup::remove_all(&paths)?
            } else {
                setup::remove_model(&paths, spec)?
            };
            if c.json {
                return write_json(
                    c.out.as_mut(),
                    &serde_json::json!({"freed": freed, "model": if fs.get_bool("all") { "" } else { spec.id }, "all": fs.get_bool("all")}),
                );
            }
            if !c.quiet {
                writeln!(
                    c.out,
                    "removed {} from {}{}",
                    size_words(freed),
                    paths.root.display(),
                    if fs.get_bool("all") {
                        " (runtime and every model)".to_string()
                    } else {
                        format!(" ({})", spec.id)
                    }
                )?;
            }
            Ok(())
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_cli::Buffer;

    fn context(home: &str, json: bool) -> (Context, Buffer, Buffer) {
        let out = Buffer::new();
        let err = Buffer::new();
        let c = Context {
            store: None,
            home: home.into(),
            out: Box::new(out.clone()),
            err: Box::new(err.clone()),
            json,
            quiet: false,
            color: false,
            agent: String::new(),
            project: String::new(),
            session: String::new(),
            now: "2026-11-01T10:00:00Z".into(),
            args: Vec::new(),
        };
        (c, out, err)
    }

    fn flagset() -> FlagSet {
        let mut fs = FlagSet::new("voice");
        flags(&mut fs);
        fs
    }

    fn sv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn status_prints_the_models_and_json_mirrors_it() {
        let home = tempfile::tempdir().unwrap();
        let (mut c, out, _) = context(home.path().to_str().unwrap(), false);
        run(&mut c, &flagset(), &[]).unwrap();
        let text = out.string();
        assert!(text.contains("nemotron-3.5-asr-streaming-0.6b"), "{text}");
        assert!(text.contains("nemotron-speech-streaming-en-0.6b"), "{text}");
        assert!(
            text.contains("* nemotron-3.5"),
            "the default is marked: {text}"
        );
        assert!(text.contains("pulse voice setup"), "{text}");
        let (mut c, out, _) = context(home.path().to_str().unwrap(), true);
        run(&mut c, &flagset(), &sv(&["status"])).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.string()).unwrap();
        assert_eq!(v["model"]["id"], catalogue::DEFAULT_MODEL);
        assert_eq!(v["ready"], false);
        assert_eq!(
            v["models"].as_array().unwrap().len(),
            catalogue::MODELS.len()
        );
    }

    #[test]
    fn bad_shapes_and_unknown_models_are_refused() {
        let home = tempfile::tempdir().unwrap();
        let (mut c, _, _) = context(home.path().to_str().unwrap(), false);
        let err = run(&mut c, &flagset(), &sv(&["dance"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("Invalid voice command"), "{err}");
        let err = run(&mut c, &flagset(), &sv(&["transcribe"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("Invalid voice command"), "{err}");
        let mut fs = flagset();
        fs.parse(&sv(&["--model", "whisper"])).unwrap();
        let err = run(&mut c, &fs, &sv(&["status"])).unwrap_err().to_string();
        assert!(err.contains("Unknown model \"whisper\""), "{err}");
        assert!(err.contains("nemotron-3.5-asr-streaming-0.6b"), "{err}");
    }

    #[test]
    fn transcribe_needs_a_readable_wav_and_a_set_up_model() {
        let home = tempfile::tempdir().unwrap();
        let (mut c, _, _) = context(home.path().to_str().unwrap(), false);
        let err = run(&mut c, &flagset(), &sv(&["transcribe", "/nonexistent.wav"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("Cannot read"), "{err}");
        let bad = home.path().join("x.wav");
        std::fs::write(&bad, b"nope").unwrap();
        let err = run(
            &mut c,
            &flagset(),
            &sv(&["transcribe", bad.to_str().unwrap()]),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("Cannot decode"), "{err}");
        // A real WAV, no model on disk: the engine says what to run.
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let wav = home.path().join("ok.wav");
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        for _ in 0..1600 {
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();
        let err = run(
            &mut c,
            &flagset(),
            &sv(&["transcribe", wav.to_str().unwrap()]),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("not set up yet") || err.contains("no ONNX Runtime build"),
            "{err}"
        );
    }

    #[test]
    fn remove_reports_what_it_freed() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::new(home.path().to_str().unwrap());
        let spec = catalogue::model_or_default("");
        let f = paths.model_file(spec, &spec.files[0]);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"123456").unwrap();
        let (mut c, out, _) = context(home.path().to_str().unwrap(), true);
        run(&mut c, &flagset(), &sv(&["remove"])).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.string()).unwrap();
        assert_eq!(v["freed"], 6);
        assert_eq!(v["model"], spec.id);
        let (mut c, out, _) = context(home.path().to_str().unwrap(), false);
        let mut fs = flagset();
        fs.parse(&sv(&["--all"])).unwrap();
        run(&mut c, &fs, &sv(&["remove"])).unwrap();
        assert!(
            out.string().contains("runtime and every model"),
            "{}",
            out.string()
        );
    }
}
