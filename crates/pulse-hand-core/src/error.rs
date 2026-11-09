//! The error a command returns when it has its own exit code: what went
//! wrong, why, what fixes it. `pulse` and `pulse-hand` print it the same way
//! and exit with its code.

use std::fmt;

#[derive(Debug, Clone)]
pub struct Error {
    pub what: String,
    pub why: String,
    pub fix: String,
    pub code: i32,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.what)?;
        if !self.why.is_empty() {
            write!(f, "\n  why: {}", self.why)?;
        }
        if !self.fix.is_empty() {
            write!(f, "\n  fix: {}", self.fix)?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

impl Error {
    pub fn exit_code(&self) -> i32 {
        if self.code == 0 { 1 } else { self.code }
    }
}

pub fn errorf(code: i32, what: &str, why: &str, fix: &str) -> Error {
    Error {
        what: what.to_string(),
        why: why.to_string(),
        fix: fix.to_string(),
        code,
    }
}
