// SPDX-License-Identifier: Apache-2.0

//! How a command's result reaches the caller: one JSON object on stdout with
//! `--json`, otherwise short text, with errors on stderr.

use std::borrow::Borrow;
use std::fmt::Display;
use std::path::Path;
use std::process::ExitCode;

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
        let code = match e {
            // Like a pack that fails `decode` validation.
            RenderError::Room(_) => "E_SCHEMA",
            RenderError::NoAdapter | RenderError::Device(_) => "E_GPU",
        };
        Failure::new(code, e.to_string())
    }
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
