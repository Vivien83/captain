use super::*;

#[test]
fn test_detect_image_format_png() {
    let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x10\x00\x00\x00\x10";
    assert_eq!(detect_image_format(data), "png");
}

#[test]
fn test_detect_image_format_jpeg() {
    let data = b"\xFF\xD8\xFF\xE0\x00\x10JFIF";
    assert_eq!(detect_image_format(data), "jpeg");
}

#[test]
fn test_detect_image_format_gif() {
    let data = b"GIF89a\x10\x00\x10\x00";
    assert_eq!(detect_image_format(data), "gif");
}

#[test]
fn test_detect_image_format_bmp() {
    let data = b"BM\x00\x00\x00\x00";
    assert_eq!(detect_image_format(data), "bmp");
}

#[test]
fn test_detect_image_format_unknown() {
    let data = b"\x00\x00\x00\x00";
    assert_eq!(detect_image_format(data), "unknown");
}

#[test]
fn test_extract_png_dimensions() {
    // Minimal PNG header: signature (8) + IHDR length (4) + "IHDR" (4) + width (4) + height (4)
    let mut data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]; // signature
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]); // IHDR length
    data.extend_from_slice(b"IHDR"); // chunk type
    data.extend_from_slice(&640u32.to_be_bytes()); // width
    data.extend_from_slice(&480u32.to_be_bytes()); // height
    assert_eq!(extract_image_dimensions(&data, "png"), Some((640, 480)));
}

#[test]
fn test_extract_gif_dimensions() {
    let mut data = b"GIF89a".to_vec();
    data.extend_from_slice(&320u16.to_le_bytes()); // width
    data.extend_from_slice(&240u16.to_le_bytes()); // height
    assert_eq!(extract_image_dimensions(&data, "gif"), Some((320, 240)));
}

#[test]
fn test_format_file_size() {
    assert_eq!(format_file_size(500), "500 B");
    assert_eq!(format_file_size(1536), "1.5 KB");
    assert_eq!(format_file_size(2 * 1024 * 1024), "2.0 MB");
}

#[tokio::test]
async fn test_image_analyze_missing_file() {
    let result = execute_tool(
        "test-id",
        "image_analyze",
        &serde_json::json!({"path": "/nonexistent/image.png"}),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("Failed to read"));
}
