use super::*;

/// v3.8d — screenshot tool is declared and reachable via dispatch.
#[test]
fn test_screenshot_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"screenshot"),
        "screenshot tool must be registered"
    );
}

#[test]
fn document_create_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let document = tools
        .iter()
        .find(|t| t.name == "document_create")
        .expect("document_create must be registered");
    assert!(
        document.description.contains("pdf") && document.description.contains("docx"),
        "document_create must advertise native document formats"
    );
    assert!(document.input_schema["properties"]["format"].is_object());
    assert!(tools.iter().any(|t| t.name == "document_extract"));
}

#[test]
fn document_extract_pdf_literal_text_stream() {
    let pdf = b"%PDF-1.4\n1 0 obj\n<< /Length 28 >>\nstream\nBT (Hello\\040Captain) Tj ET\nendstream\nendobj\n%%EOF";
    let extracted = extract_pdf_text_from_bytes(pdf).expect("simple PDF text should extract");
    assert!(extracted.text.contains("Hello Captain"), "{extracted:?}");
    assert_eq!(extracted.streams_seen, 1);
}

#[test]
fn web_download_filename_sanitizes_and_extends_pdf() {
    let filename = ensure_extension_for_mime(
        &sanitize_download_filename("weird report 2026"),
        Some("application/pdf"),
    );
    assert_eq!(filename, "weird_report_2026.pdf");
}

/// v3.8d — screenshot_command picks a sensible default per OS.
#[test]
fn test_screenshot_command_picks_per_os() {
    let picked = screenshot_command("/tmp/x.png");
    match std::env::consts::OS {
        "macos" => {
            let (cmd, args) = picked.expect("macOS must resolve to screencapture");
            assert_eq!(cmd, "screencapture");
            assert!(args.iter().any(|a| a == "/tmp/x.png"));
        }
        "linux" => {
            // May return None if no screenshot tool is installed in CI —
            // just check that *if* Some, the args include the path.
            if let Some((_, args)) = picked {
                assert!(args.iter().any(|a| a == "/tmp/x.png"));
            }
        }
        _ => {
            // Other platforms may return None; assert no panic.
            let _ = picked;
        }
    }
}
