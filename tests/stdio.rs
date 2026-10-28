use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn heading_output_aliases_cannot_overwrite_input_pdf_or_trace() {
    const INPUT: &str = "<h1>Preserved input</h1>";
    const PDF: &[u8] = b"preserved PDF";
    const TRACE: &[u8] = b"preserved trace";
    let aliases = [
        "absolute-pdf",
        "hardlink-pdf",
        "hardlink-input",
        "hardlink-trace",
        #[cfg(unix)]
        "symlink-pdf",
        #[cfg(unix)]
        "dangling-symlink-pdf",
    ];
    for (index, alias) in aliases.into_iter().enumerate() {
        let directory = std::env::temp_dir().join(format!(
            "ferri-pdf-heading-alias-{}-{index}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let input = directory.join("input.html");
        let pdf = directory.join("output.pdf");
        let trace = directory.join("output.trace.json");
        std::fs::write(&input, INPUT).unwrap();
        let missing_pdf = alias == "dangling-symlink-pdf";
        if !missing_pdf {
            std::fs::write(&pdf, PDF).unwrap();
        }
        std::fs::write(&trace, TRACE).unwrap();
        let headings = if alias == "absolute-pdf" {
            pdf.clone()
        } else {
            let path = directory.join("alias.json");
            match alias {
                #[cfg(unix)]
                "symlink-pdf" | "dangling-symlink-pdf" => {
                    std::os::unix::fs::symlink("output.pdf", &path).unwrap();
                }
                _ => {
                    let source = match alias {
                        "hardlink-pdf" => &pdf,
                        "hardlink-input" => &input,
                        "hardlink-trace" => &trace,
                        _ => unreachable!(),
                    };
                    std::fs::hard_link(source, &path).unwrap();
                }
            }
            path
        };
        let result = Command::new(env!("CARGO_BIN_EXE_ferri-pdf"))
            .args(["input.html", "output.pdf", "--trace", "--headings"])
            .arg(&headings)
            .current_dir(&directory)
            .output()
            .unwrap();
        let input_after = std::fs::read(&input).unwrap();
        let pdf_after = std::fs::read(&pdf);
        let trace_after = std::fs::read(&trace).unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        assert!(
            !result.status.success(),
            "aliased {alias} outputs must be rejected before artifact writes"
        );
        assert_eq!(input_after, INPUT.as_bytes(), "input changed for {alias}");
        if missing_pdf {
            assert_eq!(
                pdf_after.unwrap_err().kind(),
                std::io::ErrorKind::NotFound,
                "PDF was created for {alias}"
            );
        } else {
            assert_eq!(pdf_after.unwrap(), PDF, "PDF changed for {alias}");
        }
        assert_eq!(trace_after, TRACE, "trace changed for {alias}");
    }
}

#[test]
fn heading_map_can_be_written_without_trace_while_pdf_uses_stdout() {
    let directory = std::env::temp_dir().join(format!("ferri-pdf-headings-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let headings = directory.join("headings.json");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ferri-pdf"))
        .args(["-", "-", "--headings"])
        .arg(&headings)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"<!doctype html><style>@page{size:200pt 200pt;margin:10pt}h1,h2{margin:0;font:10pt/12pt serif}h2{break-before:page}</style><h1 id=first>First</h1><h2 id=second>Second</h2>").unwrap();
    let result = child.wait_with_output().unwrap();
    let metadata = std::fs::read(&headings);
    std::fs::remove_dir_all(&directory).unwrap();
    assert!(
        result.status.success(),
        "heading-map CLI failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        result.stdout.starts_with(b"%PDF-") && result.stdout.trim_ascii_end().ends_with(b"%%EOF"),
        "stdout must remain a complete PDF when heading JSON has its own destination"
    );
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.unwrap()).unwrap();
    assert_eq!(metadata.as_array().unwrap().len(), 2);
    assert_eq!(
        (
            &metadata[0]["id"],
            &metadata[0]["text"],
            &metadata[0]["level"],
            &metadata[0]["page"]
        ),
        (
            &serde_json::json!("first"),
            &serde_json::json!("First"),
            &serde_json::json!(1),
            &serde_json::json!(1)
        )
    );
    assert_eq!(
        (&metadata[1]["id"], &metadata[1]["page"]),
        (&serde_json::json!("second"), &serde_json::json!(2))
    );
}

#[test]
fn stdin_to_stdout_writes_only_pdf_bytes_and_reports_diagnostics_on_stderr() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ferri-pdf"))
        .args(["-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"<p>PDF bytes over standard IO.</p>")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        result.stdout.starts_with(b"%PDF-"),
        "stdout must start with the PDF header, not status messages"
    );
    assert!(
        result.stdout.trim_ascii_end().ends_with(b"%%EOF"),
        "stdout must contain the complete PDF"
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("1 pages"));
}

#[test]
fn relative_input_in_the_current_directory_resolves_images_beside_it() {
    const INPUT: &str = "image-block.html";
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/fixtures");
    let output =
        std::env::temp_dir().join(format!("ferri-pdf-cli-relative-{}.pdf", std::process::id()));
    let result = Command::new(env!("CARGO_BIN_EXE_ferri-pdf"))
        .arg(INPUT)
        .arg(&output)
        .current_dir(&fixtures)
        .output()
        .unwrap();
    let pdf = std::fs::read(&output);
    let _ = std::fs::remove_file(&output);
    assert!(
        result.status.success(),
        "{INPUT} has no directory component, so its images resolve against the current \
         directory: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        pdf.is_ok_and(|bytes| bytes.starts_with(b"%PDF-")),
        "the PDF must be written to {}",
        output.display()
    );
}

#[test]
fn cli_accepts_legacy_bytes_from_file_and_stdin() {
    const SOURCE: &[u8] = b"<meta charset=windows-1252><p>caf\xe9";
    let directory = std::env::temp_dir().join(format!("ferri-pdf-legacy-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("legacy.html");
    std::fs::write(&input, SOURCE).unwrap();
    let mut results = Vec::new();
    for stdin in [false, true] {
        let pdf = directory.join(if stdin { "stdin.pdf" } else { "file.pdf" });
        let mut command = Command::new(env!("CARGO_BIN_EXE_ferri-pdf"));
        command
            .arg(if stdin {
                std::path::Path::new("-")
            } else {
                &input
            })
            .arg(&pdf)
            .arg("--trace")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        if stdin {
            child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        }
        let result = child.wait_with_output().unwrap();
        let trace = std::fs::read(pdf.with_extension("trace.json"));
        results.push((stdin, result, trace));
    }
    std::fs::remove_dir_all(directory).unwrap();
    for (stdin, result, trace) in results {
        assert!(
            result.status.success(),
            "legacy byte input stdin={stdin}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let trace: serde_json::Value = serde_json::from_slice(&trace.unwrap()).unwrap();
        assert_eq!(trace["pages"][0][0]["text"], "café");
    }
}
