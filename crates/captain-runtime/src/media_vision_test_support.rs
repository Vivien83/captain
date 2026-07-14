use captain_types::media::{MediaAttachment, MediaSource, MediaType};
use tokio::sync::Mutex as AsyncMutex;

pub(super) static VISION_TEST_LOCK: AsyncMutex<()> = AsyncMutex::const_new(());

pub(super) fn temp_png_attachment(bytes: &[u8]) -> (tempfile::NamedTempFile, MediaAttachment) {
    temp_png_attachment_with_hint(bytes, None)
}

pub(super) fn temp_png_attachment_with_hint(
    bytes: &[u8],
    hint: Option<String>,
) -> (tempfile::NamedTempFile, MediaAttachment) {
    temp_png_attachment_full(bytes, hint, None)
}

pub(super) fn temp_png_attachment_full(
    bytes: &[u8],
    hint: Option<String>,
    batch_hint: Option<usize>,
) -> (tempfile::NamedTempFile, MediaAttachment) {
    use std::io::Write;
    let mut f = tempfile::Builder::new()
        .suffix(".png")
        .tempfile()
        .expect("tempfile");
    f.write_all(bytes).expect("write png bytes");
    let path = f.path().to_string_lossy().to_string();
    let att = MediaAttachment {
        media_type: MediaType::Image,
        mime_type: "image/png".into(),
        source: MediaSource::FilePath { path },
        size_bytes: bytes.len() as u64,
        context_hint: hint,
        batch_size_hint: batch_hint,
    };
    (f, att)
}
