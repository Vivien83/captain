use base64::Engine;
use captain_types::media::{GeneratedImage, ImageGenModel, ImageGenRequest, ImageGenResult};

use super::save_images_to_workspace;

#[test]
fn test_validate_valid_request() {
    let req = ImageGenRequest {
        prompt: "A beautiful sunset".to_string(),
        model: ImageGenModel::DallE3,
        size: "1024x1024".to_string(),
        quality: "hd".to_string(),
        count: 1,
    };
    assert!(req.validate().is_ok());
}

#[test]
fn test_validate_empty_prompt() {
    let req = ImageGenRequest {
        prompt: String::new(),
        model: ImageGenModel::DallE3,
        size: "1024x1024".to_string(),
        quality: "standard".to_string(),
        count: 1,
    };
    assert!(req.validate().is_err());
}

#[test]
fn test_validate_dalle2_sizes() {
    for size in &["256x256", "512x512", "1024x1024"] {
        let req = ImageGenRequest {
            prompt: "test".to_string(),
            model: ImageGenModel::DallE2,
            size: size.to_string(),
            quality: "standard".to_string(),
            count: 1,
        };
        assert!(req.validate().is_ok(), "Failed for size {size}");
    }
}

#[test]
fn test_validate_gpt_image_sizes() {
    for size in &["1024x1024", "1536x1024", "1024x1536", "auto"] {
        let req = ImageGenRequest {
            prompt: "test".to_string(),
            model: ImageGenModel::GptImage1,
            size: size.to_string(),
            quality: "auto".to_string(),
            count: 2,
        };
        assert!(req.validate().is_ok(), "Failed for size {size}");
    }
}

#[test]
fn test_save_images_creates_dir() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();
    let result = ImageGenResult {
        images: vec![GeneratedImage {
            data_base64: base64::engine::general_purpose::STANDARD.encode([0u8; 8]),
            url: None,
        }],
        model: "dall-e-3".to_string(),
        revised_prompt: None,
    };
    let paths = save_images_to_workspace(&result, workspace).unwrap();
    assert_eq!(paths.len(), 1);
    assert!(std::path::Path::new(&paths[0]).exists());
}
