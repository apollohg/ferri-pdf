//! Compiles the CSS property grammars in `spec/css` into static tables. A malformed or
//! unresolved production fails the build.
#![expect(
    clippy::print_stderr,
    reason = "build scripts report to Cargo on stdout and stderr"
)]
#[path = "src/style/angles.rs"]
mod angles;
#[path = "build/compile.rs"]
mod compile;
#[path = "build/emit.rs"]
mod emit;
#[path = "build/source.rs"]
mod source;
#[path = "build/syntax.rs"]
mod syntax;

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

const BUILD_SOURCES: [&str; 2] = ["build.rs", "build"];
const OUTPUT_FILE: &str = "grammar.rs";
const SPEC_CSS_DIRECTORY: &str = "spec/css";

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
    for source in BUILD_SOURCES {
        println!("cargo:rerun-if-changed={source}");
    }
    let spec = source::Spec::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join(SPEC_CSS_DIRECTORY))?;
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
