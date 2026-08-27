use std::{collections::HashSet, path::PathBuf, time::Duration};

use fast_image_resize::{IntoImageView, ResizeOptions, Resizer, images::Image};
use image::{ImageDecoder, ImageReader, Limits};

use crate::{Error, Result};

/// Widths generated for every upload, largest last. Steps wider than the
/// original are skipped so we never upscale.
const WIDTH_LADDER: [u32; 4] = [400, 800, 1600, 2400];

const WEBP_QUALITY: f32 = 80.0;

const MAX_DIMENSION: u32 = 12_000;
const MAX_ALLOC: u64 = 256 * 1024 * 1024;

/// How long a blob is left alone before the sweep may reclaim it. An upload
/// that has been made but not yet submitted is referenced only by the open
/// compose form, so anything newer than this is assumed to be in flight.
const GC_MIN_AGE: Duration = Duration::from_secs(60 * 60);

/// Cheap to clone -- it is a path and nothing else -- so the state that needs
/// it and the services that consult it can each hold one.
#[derive(Clone)]
pub struct ImageService {
    images_dir: PathBuf,
}

pub struct ImageUpload {
    pub hash: String,
    pub width: u32,
    pub height: u32,
}

pub struct SourceSetEntry {
    pub url: String,
    pub width: u32,
}

impl ImageService {
    pub fn new(images_dir: PathBuf) -> Self {
        ImageService { images_dir }
    }

    pub fn run_blob_gc(&self, referenced: &HashSet<String>) -> Result<usize> {
        let mut reclaimed = 0;

        for entry in std::fs::read_dir(&self.images_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let name = entry.file_name();
            let Some(hash) = name.to_str() else {
                continue;
            };

            if !is_blob_hash(hash) || referenced.contains(hash) {
                continue;
            }

            let age = entry
                .metadata()?
                .modified()?
                .elapsed()
                .unwrap_or(Duration::ZERO);
            if age < GC_MIN_AGE {
                continue;
            }

            std::fs::remove_dir_all(entry.path())?;
            tracing::info!(hash, "reclaimed unreferenced blob");
            reclaimed += 1;
        }

        return Ok(reclaimed);
    }

    pub fn exists(&self, hash: &str) -> bool {
        return is_blob_hash(hash) && self.images_dir.join(hash).is_dir();
    }

    /// Decoding, resizing and encoding are entirely CPU-bound and take on the
    /// order of a second per photo. Run directly they would occupy a runtime
    /// worker for that whole time, stalling every other request scheduled on
    /// it, so the work goes to the blocking pool instead.
    pub async fn upload_image(&self, image_bytes: Vec<u8>) -> Result<ImageUpload> {
        let service = self.clone();

        return tokio::task::spawn_blocking(move || service.process(&image_bytes))
            .await
            .map_err(|err| Error::Io(std::io::Error::other(err)))?;
    }

    fn process(&self, image_bytes: &[u8]) -> Result<ImageUpload> {
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        limits.max_alloc = Some(MAX_ALLOC);

        let mut reader =
            ImageReader::new(std::io::Cursor::new(image_bytes)).with_guessed_format()?;
        reader.limits(limits);

        // Orientation must come off the decoder before the pixels are taken:
        // decoding drops the metadata, and re-encoding never carries it over.
        let mut decoder = reader.into_decoder()?;
        let orientation = decoder.orientation()?;
        let mut decoded = image::DynamicImage::from_decoder(decoder)?;
        decoded.apply_orientation(orientation);

        // Measured after rotating, so they describe what a browser lays out.
        let source = image::DynamicImage::ImageRgb8(decoded.to_rgb8());
        let width = source.width();
        let height = source.height();

        let hash = blake3::hash(image_bytes).to_hex().to_string();
        let dir = self.images_dir.join(&hash);
        std::fs::create_dir_all(&dir)?;

        let pixel_type = source.pixel_type().ok_or(Error::UnsupportedImage)?;
        let mut resizer = Resizer::new();

        for target_width in ladder_for(width) {
            let target_height = scale_height(width, height, target_width);

            let mut dst = Image::new(target_width, target_height, pixel_type);
            resizer.resize(&source, &mut dst, &ResizeOptions::new())?;

            let encoded = webp::Encoder::from_rgb(dst.buffer(), target_width, target_height)
                .encode(WEBP_QUALITY);

            std::fs::write(dir.join(format!("{target_width}.webp")), &*encoded)?;
        }

        return Ok(ImageUpload {
            hash,
            width,
            height,
        });
    }

    pub fn derive_sourceset(&self, hash: &str, original_width: u32) -> Vec<SourceSetEntry> {
        return ladder_for(original_width)
            .map(|width| SourceSetEntry {
                url: format!("/img/{hash}/{width}.webp"),
                width,
            })
            .collect();
    }
}

// used to ensure hash is blob hash to only delete dirs that are owned by this
fn is_blob_hash(hash: &str) -> bool {
    return hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
}

fn ladder_for(original_width: u32) -> impl Iterator<Item = u32> {
    // Two ceilings. Anything wider than the source would be upscaling, and
    // anything past the widest rung is more pixels than the layout can use --
    // a full-resolution phone photo costs more to encode than every other
    // variant combined and is never the one a browser picks.
    let widest = original_width.min(WIDTH_LADDER[WIDTH_LADDER.len() - 1]);

    return WIDTH_LADDER
        .into_iter()
        .filter(move |w| *w < widest)
        .chain(std::iter::once(widest));
}

fn scale_height(src_width: u32, src_height: u32, dst_width: u32) -> u32 {
    return ((src_height as u64 * dst_width as u64) / src_width as u64).max(1) as u32;
}

// Claude written
#[cfg(test)]
mod tests {
    use super::*;

    /// A JPEG carrying a single EXIF Orientation tag, built by hand so the
    /// test doesn't need a binary fixture.
    fn jpeg_with_orientation(width: u32, height: u32, orientation: u16) -> Vec<u8> {
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(width, height))
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();

        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II*\0");
        tiff.extend_from_slice(&8u32.to_le_bytes()); // offset of IFD0
        tiff.extend_from_slice(&1u16.to_le_bytes()); // one entry
        tiff.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
        tiff.extend_from_slice(&3u16.to_le_bytes()); // SHORT
        tiff.extend_from_slice(&1u32.to_le_bytes()); // count
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]); // value field padded to 4 bytes
        tiff.extend_from_slice(&0u32.to_le_bytes()); // no next IFD

        let mut app1 = Vec::from(*b"Exif\0\0");
        app1.extend_from_slice(&tiff);

        let mut out = Vec::new();
        out.extend_from_slice(&jpeg[..2]); // SOI
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&app1);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    fn upload(bytes: &[u8]) -> (tempfile::TempDir, ImageService, ImageUpload) {
        let dir = tempfile::tempdir().unwrap();
        let service = ImageService::new(dir.path().to_path_buf());
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let uploaded = rt.block_on(service.upload_image(bytes.to_vec())).unwrap();
        (dir, service, uploaded)
    }

    #[test]
    fn applies_exif_orientation() {
        let (_dir, _service, uploaded) = upload(&jpeg_with_orientation(1000, 500, 6));

        // Orientation 6 means "rotate 90", so the landscape source is
        // displayed as a portrait and the dimensions swap.
        assert_eq!((uploaded.width, uploaded.height), (500, 1000));
    }

    #[test]
    fn leaves_unoriented_images_alone() {
        let (_dir, _service, uploaded) = upload(&jpeg_with_orientation(1000, 500, 1));

        assert_eq!((uploaded.width, uploaded.height), (1000, 500));
    }

    #[test]
    fn strips_exif_from_written_variants() {
        let (dir, service, uploaded) = upload(&jpeg_with_orientation(1000, 500, 6));

        for entry in service.derive_sourceset(&uploaded.hash, uploaded.width) {
            let file = dir
                .path()
                .join(&uploaded.hash)
                .join(format!("{}.webp", entry.width));
            let written = std::fs::read(&file).unwrap();

            assert!(
                written.windows(4).all(|w| w != b"Exif"),
                "{file:?} still carries an EXIF chunk"
            );
        }
    }

    #[test]
    fn ladder_stops_at_the_widest_rung() {
        // A phone photo gets the four rungs and nothing at full resolution.
        let steps: Vec<u32> = ladder_for(4032).collect();
        assert_eq!(steps, WIDTH_LADDER);

        // Below the top rung the original is still the last step, so smaller
        // images keep an exact-size variant.
        assert_eq!(ladder_for(900).collect::<Vec<u32>>(), [400, 800, 900]);
        assert_eq!(ladder_for(2400).collect::<Vec<u32>>(), WIDTH_LADDER);
    }

    #[test]
    fn ladder_never_upscales() {
        for original in [300u32, 400, 900, 1000, 3000] {
            let steps: Vec<u32> = ladder_for(original).collect();

            assert!(!steps.is_empty());
            assert!(
                steps.iter().all(|s| *s <= original),
                "{original} -> {steps:?}"
            );
        }
    }
}
