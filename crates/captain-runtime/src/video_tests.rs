use super::*;

fn temp_subdir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("captain_video_test_{label}_{}", std::process::id()))
}

#[test]
fn config_default_is_sensible() {
    let cfg = VideoFrameExtractConfig::default();
    assert!((cfg.fps - 1.0).abs() < f32::EPSILON);
    assert_eq!(cfg.max_frames, 30);
    assert!(cfg.output_dir.starts_with(std::env::temp_dir()));
}

#[tokio::test]
async fn extract_frames_errs_when_video_missing() {
    let cfg = VideoFrameExtractConfig {
        output_dir: temp_subdir("missing_out"),
        ..Default::default()
    };
    let err = extract_frames(Path::new("/tmp/captain_no_such_video_xyz_184.mp4"), &cfg)
        .await
        .expect_err("expected err for missing video");
    assert!(err.contains("not found"), "got: {err}");
}

#[tokio::test]
async fn extract_frames_errs_when_max_frames_zero() {
    let video = temp_subdir("max0").with_extension("mp4");
    std::fs::write(&video, b"fake mp4 marker").unwrap();
    let cfg = VideoFrameExtractConfig {
        fps: 1.0,
        max_frames: 0,
        output_dir: temp_subdir("max0_out"),
    };
    let err = extract_frames(&video, &cfg)
        .await
        .expect_err("expected err for max_frames=0");
    assert!(err.contains("max_frames"), "got: {err}");
    let _ = std::fs::remove_file(&video);
}

#[tokio::test]
async fn extract_frames_errs_when_fps_invalid() {
    let video = temp_subdir("fps").with_extension("mp4");
    std::fs::write(&video, b"fake mp4 marker").unwrap();
    let base = VideoFrameExtractConfig {
        fps: 0.0,
        max_frames: 10,
        output_dir: temp_subdir("fps_out"),
    };

    let err0 = extract_frames(&video, &base)
        .await
        .expect_err("expected err for fps=0");
    assert!(err0.contains("fps"), "got: {err0}");

    let nan_cfg = VideoFrameExtractConfig {
        fps: f32::NAN,
        ..base.clone()
    };
    let err_nan = extract_frames(&video, &nan_cfg)
        .await
        .expect_err("expected err for fps=NaN");
    assert!(err_nan.contains("fps"), "got: {err_nan}");

    let neg_cfg = VideoFrameExtractConfig { fps: -1.0, ..base };
    let err_neg = extract_frames(&video, &neg_cfg)
        .await
        .expect_err("expected err for fps<0");
    assert!(err_neg.contains("fps"), "got: {err_neg}");

    let _ = std::fs::remove_file(&video);
}

#[test]
fn downscale_shrinks_large_image() {
    let dir = temp_subdir("downscale_large");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("big.png");

    let img = image::DynamicImage::new_rgb8(2000, 2000);
    img.save(&path).expect("save synth png");

    downscale_to_long_edge(&path, VISION_MAX_LONG_EDGE).expect("downscale failed");

    let out = image::open(&path).expect("re-open downscaled");
    assert_eq!(out.width().max(out.height()), VISION_MAX_LONG_EDGE);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn downscale_noop_when_already_small() {
    let dir = temp_subdir("downscale_small");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("small.png");

    let img = image::DynamicImage::new_rgb8(800, 600);
    img.save(&path).expect("save synth png");
    let before = std::fs::metadata(&path).unwrap().len();

    downscale_to_long_edge(&path, VISION_MAX_LONG_EDGE).expect("downscale failed");

    let after = std::fs::metadata(&path).unwrap().len();
    assert_eq!(before, after);

    let out = image::open(&path).expect("re-open");
    assert_eq!(out.width(), 800);
    assert_eq!(out.height(), 600);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn downscale_preserves_aspect_ratio() {
    let dir = temp_subdir("downscale_aspect");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("wide.png");

    let img = image::DynamicImage::new_rgb8(2000, 1000);
    img.save(&path).expect("save synth png");

    downscale_to_long_edge(&path, 1024).expect("downscale failed");

    let out = image::open(&path).expect("re-open");
    assert_eq!(out.width(), 1024);
    assert_eq!(out.height(), 512);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn extract_audio_errs_when_video_missing() {
    let out = temp_subdir("audio_missing").join("out.mp3");
    let err = extract_audio(Path::new("/tmp/captain_no_such_audio_xyz_184.mp4"), &out)
        .await
        .expect_err("expected err for missing video");
    assert!(
        err.contains("not found"),
        "expected 'not found' in err, got: {err}"
    );
}

#[tokio::test]
async fn extract_audio_creates_parent_dir() {
    let video = temp_subdir("audio_parent").with_extension("mp4");
    std::fs::write(&video, b"fake mp4 marker").unwrap();
    let parent = temp_subdir("audio_parent_out").join("nested").join("dir");
    let out = parent.join("audio.mp3");

    let _ = extract_audio(&video, &out).await;
    assert!(
        parent.exists() && parent.is_dir(),
        "parent dir should have been created: {}",
        parent.display()
    );

    let _ = std::fs::remove_file(&video);
    let _ = std::fs::remove_dir_all(temp_subdir("audio_parent_out"));
}

#[tokio::test]
async fn transcode_audio_to_wav_rejects_identical_input_and_output() {
    let dir = temp_subdir("wav_same_path");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("input.wav");
    std::fs::write(&path, b"fake wav marker").unwrap();

    let err = transcode_audio_to_wav(&path, &path)
        .await
        .expect_err("expected explicit same-path rejection");

    assert!(
        err.contains("input and output path are identical"),
        "got: {err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore]
async fn extracts_audio_from_testsrc() {
    ensure_ffmpeg().await.expect("ensure_ffmpeg failed");

    let dir = temp_subdir("audio_happy");
    std::fs::create_dir_all(&dir).unwrap();
    let video = dir.join("withaudio.mp4");
    let video_str = video.to_string_lossy().to_string();

    tokio::task::spawn_blocking(move || {
        FfmpegCommand::new()
            .args([
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=3:size=320x240:rate=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=3",
                "-shortest",
                "-y",
            ])
            .output(&video_str)
            .spawn()
            .expect("spawn testsrc")
            .wait()
            .expect("wait testsrc");
    })
    .await
    .unwrap();

    let mp3 = dir.join("out.mp3");
    extract_audio(&video, &mp3)
        .await
        .expect("extract_audio failed");
    let meta = std::fs::metadata(&mp3).expect("mp3 missing");
    assert!(meta.len() > 0, "mp3 is empty");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore]
async fn extracts_frames_from_testsrc() {
    ensure_ffmpeg().await.expect("ensure_ffmpeg failed");

    let dir = temp_subdir("happy");
    std::fs::create_dir_all(&dir).unwrap();
    let video = dir.join("testsrc.mp4");
    let video_str = video.to_string_lossy().to_string();

    tokio::task::spawn_blocking(move || {
        FfmpegCommand::new()
            .args([
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=5:size=320x240:rate=10",
                "-y",
            ])
            .output(&video_str)
            .spawn()
            .expect("spawn testsrc")
            .wait()
            .expect("wait testsrc");
    })
    .await
    .unwrap();

    let cfg = VideoFrameExtractConfig {
        fps: 1.0,
        max_frames: 5,
        output_dir: dir.join("frames"),
    };
    let frames = extract_frames(&video, &cfg)
        .await
        .expect("extract_frames failed");
    assert!(!frames.is_empty(), "expected at least 1 frame");
    assert!(
        frames.len() <= 5,
        "max_frames not respected: {}",
        frames.len()
    );
    for p in &frames {
        assert!(p.exists(), "frame absent: {}", p.display());
        assert_eq!(p.extension().and_then(|s| s.to_str()), Some("jpg"));
    }

    let _ = std::fs::remove_dir_all(&dir);
}
