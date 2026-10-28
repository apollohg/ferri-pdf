//! Compiles the CSS property grammars in `spec/css` into static tables. A malformed or
//! unresolved production fails the build.
#[path = "../src/angles.rs"]
mod angles;
mod compile;
mod emit;
mod source;
mod syntax;

use std::{env, fs, path::PathBuf, process::ExitCode};

const BUILD_DIRECTORY: &str = "build";
const OUTPUT_FILE: &str = "grammar.rs";

#[derive(Debug, thiserror::Error)]
pub(crate) enum BuildError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Json {
        path: String,
        source: serde_json::Error,
    },
    #[error("{context}: missing or invalid `{field}`")]
    Field {
        context: String,
        field: &'static str,
    },
    #[error("{context}: {error} in `{text}`")]
    Syntax {
        context: String,
        text: String,
        error: syntax::SyntaxError,
    },
    #[error("{context}: unresolved production <{name}>")]
    Unresolved { context: String, name: String },
    #[error("erratum for unknown property {name}")]
    UnknownErratum { name: String },
    #[error("extension {name} duplicates a specification property")]
    DuplicateExtension { name: String },
    #[error("erratum adds {name}, which the data already defines")]
    DuplicateAddition { name: String },
    #[error("{context}: {message}")]
    Invalid { context: String, message: String },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("CSS grammar compilation failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), BuildError> {
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("set by cargo")).join(OUTPUT_FILE);
    println!("cargo:rerun-if-changed={BUILD_DIRECTORY}");
    let spec = source::Spec::load(&ferri_pdf_spec_data::css_directory())?;
    for file in &spec.files {
        println!("cargo:rerun-if-changed={file}");
    }
    let grammar = compile::compile(&spec)?;
    let code = emit::emit(&grammar)?;
    fs::write(&output, code).map_err(|source| BuildError::Io {
        path: output.display().to_string(),
        source,
    })
}
