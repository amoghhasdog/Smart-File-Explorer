//! Thumbnail cache: `<thumbs_dir>/<file_id>.jpg`, longest side ≤ 512 px.
//! Generated in the text stage for images; served to the UI by the app
//! shell and fed to the image embedder (CLIP only needs 224 px).

use std::io::BufReader;
use std::path::{Path, PathBuf};

use image::ImageReader;
use sfe_core::FileId;

pub const MAX_SIDE: u32 = 512;

pub fn path(dir: &Path, id: FileId) -> PathBuf {
    dir.join(format!("{}.jpg", id.0))
}

/// Decode `src`, downscale and write a JPEG thumbnail. Errors are returned
/// to the caller to log; thumbnails are best-effort.
pub fn generate(src: &Path, dst: &Path, max_side: u32) -> std::io::Result<()> {
    let file = sfe_fs::open_read(src).map_err(std::io::Error::other)?;
    let img = ImageReader::new(BufReader::new(file))
        .with_guessed_format()?
        .decode()
        .map_err(std::io::Error::other)?;
    let thumb = img.thumbnail(max_side, max_side).to_rgb8();
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = dst.with_extension("jpg.tmp");
    thumb
        .save_with_format(&tmp, image::ImageFormat::Jpeg)
        .map_err(std::io::Error::other)?;
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

/// Load a thumbnail (or, failing that, the original) for embedding.
pub fn load_for_embedding(
    thumb: Option<&Path>,
    original: &Path,
) -> std::io::Result<image::DynamicImage> {
    let src = match thumb {
        Some(t) if t.exists() => t,
        _ => original,
    };
    let file = sfe_fs::open_read(src).map_err(std::io::Error::other)?;
    ImageReader::new(BufReader::new(file))
        .with_guessed_format()?
        .decode()
        .map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_a_downscaled_jpeg_and_loads_it() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("big.png");
        image::RgbImage::from_pixel(1600, 800, image::Rgb([0, 0, 255]))
            .save(&src)
            .unwrap();
        let dst = path(&dir.path().join("thumbs"), FileId(7));
        generate(&src, &dst, 512).unwrap();
        assert!(dst.ends_with("7.jpg"));
        let t = image::open(&dst).unwrap();
        assert_eq!((t.width(), t.height()), (512, 256));
        let loaded = load_for_embedding(Some(&dst), &src).unwrap();
        assert_eq!(loaded.width(), 512);
        let fallback = load_for_embedding(Some(Path::new("nope.jpg")), &src).unwrap();
        assert_eq!(fallback.width(), 1600);
        assert!(generate(&dir.path().join("missing.png"), &dst, 512).is_err());
    }
}
