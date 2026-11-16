// SPDX-License-Identifier: Apache-2.0

//! How a command's result reaches the caller: one JSON object on stdout with
//! `--json`, otherwise short text, with errors on stderr.

use std::borrow::Borrow;
use std::ffi::OsString;
use std::fmt::Display;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::process::ExitCode;

use clap::error::ErrorKind;
use sariema_import::ImportError;
use sariema_render::RenderError;
use serde_json::{Value, json};

pub struct Done {
    /// The fields of the JSON reply besides `"ok"`.
    pub json: Value,
    pub text: String,
}

pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

impl Failure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Failure {
        Failure {
            code,
            message: message.into(),
        }
    }

    pub fn io(path: &Path, e: impl Display) -> Failure {
        Failure::new("E_IO", format!("{}: {e}", path.display()))
    }
}

impl From<ImportError> for Failure {
    fn from(e: ImportError) -> Failure {
        Failure::new(e.code(), e.to_string())
    }
}

impl From<RenderError> for Failure {
    fn from(e: RenderError) -> Failure {
        Failure::new(e.code(), e.to_string())
    }
}

impl From<sariema_view::ViewError> for Failure {
    fn from(e: sariema_view::ViewError) -> Failure {
        Failure::new(e.code, e.message)
    }
}

// wgpu panics with the first two on a device error nothing captured and on a
// failed poll; the renderer's read-back panics with the last two.
const GPU_PANICS: [&str; 4] = [
    "wgpu error: ",
    "Error in Device::poll",
    "waiting for the GPU",
    "mapping a read-back buffer",
];

/// Runs `command`, turning a panic into `E_GPU` when it reports a GPU device
/// error and into `E_INTERNAL` otherwise. The default hook still prints the
/// panic to stderr.
pub fn guarded(command: impl FnOnce() -> Result<Done, Failure>) -> Result<Done, Failure> {
    panic::catch_unwind(AssertUnwindSafe(command)).unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panicked without a message".into());
        let code = if GPU_PANICS.iter().any(|p| message.starts_with(p)) {
            "E_GPU"
        } else {
            "E_INTERNAL"
        };
        Err(Failure::new(code, message.trim_end()))
    })
}

/// Whether `--json` is among the arguments as a flag: after `--` every
/// argument is positional.
pub fn json_flag(args: impl IntoIterator<Item = OsString>) -> bool {
    args.into_iter()
        .take_while(|a| a != "--")
        .any(|a| a == "--json")
}

/// Reports arguments clap rejected. Help and version are successes.
pub fn parse_error(e: clap::Error, json: bool) -> ExitCode {
    if !json {
        let _ = e.print();
        return if e.use_stderr() {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        };
    }
    let outcome = match e.kind() {
        ErrorKind::DisplayHelp => Ok(json!({ "help": e.render().to_string() })),
        ErrorKind::DisplayVersion => Ok(json!({ "version": env!("CARGO_PKG_VERSION") })),
        _ => {
            // The first paragraph names the problem; usage and tips follow.
            let rendered = e.render().to_string();
            let lines: Vec<&str> = rendered
                .lines()
                .map(str::trim)
                .take_while(|l| !l.is_empty())
                .collect();
            let message = lines.join(" ");
            let message = message.strip_prefix("error: ").unwrap_or(&message);
            Err(Failure::new("E_USAGE", message))
        }
    };
    let done = outcome.map(|json| Done {
        json,
        text: String::new(),
    });
    emit(true, done)
}

pub fn emit(json: bool, outcome: Result<Done, Failure>) -> ExitCode {
    match outcome {
        Ok(mut done) => {
            if json {
                done.json["ok"] = Value::Bool(true);
                println!("{}", done.json);
            } else {
                println!("{}", done.text);
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            if json {
                let error = json!({ "code": failure.code, "message": failure.message });
                println!("{}", json!({ "ok": false, "error": error }));
            } else {
                eprintln!("error[{}]: {}", failure.code, failure.message);
            }
            ExitCode::from(1)
        }
    }
}

/// Names separated by commas, or "none".
pub fn list<S: Borrow<str>>(names: &[S]) -> String {
    if names.is_empty() {
        return "none".into();
    }
    names.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(command: impl FnOnce() -> Result<Done, Failure>) -> (&'static str, String) {
        let failure = guarded(command).err().expect("a failure");
        (failure.code, failure.message)
    }

    #[test]
    fn wgpu_device_error_panic_is_e_gpu() {
        let (code, message) = code_of(|| panic!("wgpu error: Validation Error\n\nCaused by: x\n"));
        assert_eq!(code, "E_GPU");
        assert_eq!(message, "wgpu error: Validation Error\n\nCaused by: x");
    }

    #[test]
    fn other_panics_are_e_internal() {
        assert_eq!(code_of(|| panic!("index out of bounds")).0, "E_INTERNAL");
        let why = String::from("formatted");
        assert_eq!(
            code_of(|| panic!("{why}")),
            ("E_INTERNAL", "formatted".to_string())
        );
    }

    #[test]
    fn json_flag_stops_at_double_dash() {
        let args = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(json_flag(args(&["sariema", "inspect", "x", "--json"])));
        assert!(!json_flag(args(&["sariema", "inspect", "--", "--json"])));
    }
}
