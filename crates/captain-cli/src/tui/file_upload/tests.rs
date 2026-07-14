use super::{
    parse_dropped_path, prepare_upload, upload_content_type_for_extension,
    upload_content_type_for_path, SUPPORTED_UPLOAD_FORMATS, UPLOAD_USAGE,
};

fn write_tmp(name: &str, ext: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("captain_drop_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.{ext}"));
    std::fs::write(&path, b"x").unwrap();
    path
}

#[test]
fn upload_content_type_matches_daemon_allowlist_extensions() {
    assert_eq!(upload_content_type_for_extension("png"), Some("image/png"));
    assert_eq!(
        upload_content_type_for_extension("JPEG"),
        Some("image/jpeg")
    );
    assert_eq!(
        upload_content_type_for_extension("pdf"),
        Some("application/pdf")
    );
    assert_eq!(
        upload_content_type_for_extension("md"),
        Some("text/markdown")
    );
    assert_eq!(upload_content_type_for_extension("mp3"), Some("audio/mpeg"));
    assert_eq!(upload_content_type_for_extension("bin"), None);
}

#[test]
fn upload_content_type_reads_path_extension() {
    let p = write_tmp("content-type", "csv");
    assert_eq!(upload_content_type_for_path(&p), Some("text/csv"));
}

#[test]
fn prepare_upload_reads_file_and_metadata() {
    let p = write_tmp("prepared", "md");
    let upload = prepare_upload(&p.to_string_lossy()).unwrap();

    assert_eq!(upload.path, p);
    assert_eq!(upload.filename, "prepared.md");
    assert_eq!(upload.content_type, "text/markdown");
    assert_eq!(upload.bytes, b"x");
}

#[test]
fn prepare_upload_reports_usage_for_empty_path() {
    let err = prepare_upload("").unwrap_err();

    assert_eq!(err, UPLOAD_USAGE);
}

#[test]
fn prepare_upload_reports_missing_file_with_resolved_path() {
    let err = prepare_upload("/tmp/captain_missing_upload_test.png").unwrap_err();

    assert!(err.contains("Lecture impossible (/tmp/captain_missing_upload_test.png):"));
}

#[test]
fn prepare_upload_reports_unsupported_format_with_allowlist() {
    let p = write_tmp("unsupported", "rar");
    let err = prepare_upload(&p.to_string_lossy()).unwrap_err();

    assert!(err.contains("Format non supporté (rar)."));
    assert!(err.contains(SUPPORTED_UPLOAD_FORMATS));
}

#[test]
fn drop_bare_absolute_image_path_recognised() {
    let p = write_tmp("a", "png");
    let raw = p.to_string_lossy().into_owned();
    let parsed = parse_dropped_path(&raw).unwrap();
    assert_eq!(parsed, p);
}

#[test]
fn drop_quoted_path_strips_quotes() {
    let p = write_tmp("b", "jpg");
    let raw = format!("'{}'", p.display());
    let parsed = parse_dropped_path(&raw).unwrap();
    assert_eq!(parsed, p);
}

#[test]
fn drop_escaped_spaces_are_unescaped() {
    let p = write_tmp("c", "webp");
    let raw = p.to_string_lossy().replace(' ', "\\ ");
    let parsed = parse_dropped_path(&raw).unwrap();
    assert_eq!(parsed, p);
}

#[test]
fn drop_file_url_is_recognised() {
    let p = write_tmp("d", "gif");
    let raw = format!("file://{}", p.display());
    let parsed = parse_dropped_path(&raw).unwrap();
    assert_eq!(parsed, p);
}

#[test]
fn drop_text_paste_is_ignored() {
    assert!(parse_dropped_path("hello world").is_none());
    assert!(parse_dropped_path("Bonjour Captain.").is_none());
    assert!(parse_dropped_path("https://example.com/img.png").is_none());
}

#[test]
fn drop_unknown_extension_is_ignored() {
    let p = write_tmp("e", "rar");
    let raw = p.to_string_lossy().into_owned();
    assert!(parse_dropped_path(&raw).is_none());
}

#[test]
fn drop_missing_file_is_ignored() {
    assert!(parse_dropped_path("/tmp/captain_does_not_exist_xyz.png").is_none());
}

#[test]
fn drop_directory_is_ignored() {
    let dir = std::env::temp_dir().join("captain_drop_test_dir");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(parse_dropped_path(&dir.to_string_lossy()).is_none());
    let _ = std::fs::remove_dir(&dir);
}

#[test]
fn drop_multiline_paste_is_ignored() {
    let p = write_tmp("f", "png");
    let raw = format!("{}\nsecond line", p.display());
    assert!(parse_dropped_path(&raw).is_none());
}
