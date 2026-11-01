//! Speech to text on this machine, for the hub's microphone button and
//! `pulse voice`. The models are NVIDIA's Nemotron streaming ASR (the ones
//! the GitHub Copilot CLI's `/voice` uses) run through ONNX Runtime, both
//! fetched on first use into `PULSE_HOME/voice` and checked against the
//! hashes in [`catalogue`]. No audio leaves the machine.

pub mod audio;
pub mod catalogue;
pub mod command;
pub mod download;
pub mod engine;
pub mod paths;
pub mod setup;

pub use command::{LANGUAGE_SETTING, MODEL_SETTING, register};
pub use engine::{Engine, Transcript};
pub use paths::Paths;
pub use setup::{Setup, SetupSnapshot, Status};
