//! Reproducible application asset generation for rdesktop projects.
//!
//! The public entry point turns one transparent PNG source into the common
//! Windows and desktop icon assets. The generated ICO stores real PNG
//! payloads for each size, preserving alpha and high-resolution artwork.

use std::fs;
use std::path::{Path, PathBuf};

use image::{imageops, DynamicImage, ImageFormat, Rgba, RgbaImage};

/// Sizes included in generated Windows ICO files and PNG variants.
pub const ICON_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256];

/// Maximum source edge accepted by the generator.
pub const MAX_SOURCE_EDGE: u32 = 8192;

/// The files emitted by [`generate_icons`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedIcons {
    /// The generated ICO containing all [`ICON_SIZES`] entries.
    pub ico: PathBuf,
    /// Generated square PNG variants keyed by pixel edge length.
    pub pngs: Vec<(u32, PathBuf)>,
}

/// Generate a multi-size ICO and square PNG variants from one PNG source.
///
/// Non-square sources are fitted into a transparent square canvas, preserving
/// the complete artwork rather than silently cropping it. The output name is
/// a file stem only (for example `app`), not a path.
pub fn generate_icons(
    input_png: impl AsRef<Path>,
    output_dir: impl AsRef<Path>,
    name: &str,
) -> Result<GeneratedIcons, String> {
    let input_png = input_png.as_ref();
    let output_dir = output_dir.as_ref();
    validate_name(name)?;

    let source = image::open(input_png)
        .map_err(|error| format!("failed to read PNG '{}': {error}", input_png.display()))?;
    let source = source.to_rgba8();
    validate_source(&source, input_png)?;
    fs::create_dir_all(output_dir).map_err(|error| {
        format!(
            "failed to create icon output directory '{}': {error}",
            output_dir.display()
        )
    })?;

    let mut pngs = Vec::with_capacity(ICON_SIZES.len());
    let mut ico_entries = Vec::with_capacity(ICON_SIZES.len());
    for &size in ICON_SIZES {
        let rendered = render_square(&source, size);
        let png = encode_png(&rendered, size)?;
        let png_path = output_dir.join(format!("{name}-{size}.png"));
        write_file(&png_path, &png)?;
        ico_entries.push((size, png));
        pngs.push((size, png_path));
    }

    let ico_path = output_dir.join(format!("{name}.ico"));
    write_file(&ico_path, &encode_ico(&ico_entries)?)?;

    Ok(GeneratedIcons {
        ico: ico_path,
        pngs,
    })
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("icon name must not be empty, '.' or '..'".to_string());
    }
    if name.contains('/')
        || name.contains('\\')
        || Path::new(name).file_name() != Some(name.as_ref())
    {
        return Err(format!("icon name must be a file name, got '{name}'"));
    }
    Ok(())
}

fn validate_source(source: &RgbaImage, input_png: &Path) -> Result<(), String> {
    let (width, height) = source.dimensions();
    if width == 0 || height == 0 {
        return Err(format!("PNG '{}' has no pixels", input_png.display()));
    }
    if width > MAX_SOURCE_EDGE || height > MAX_SOURCE_EDGE {
        return Err(format!(
            "PNG '{}' is {}x{}; maximum supported edge is {}",
            input_png.display(),
            width,
            height,
            MAX_SOURCE_EDGE
        ));
    }
    Ok(())
}

fn render_square(source: &RgbaImage, size: u32) -> RgbaImage {
    let (width, height) = source.dimensions();
    let scale = (size as f32 / width as f32).min(size as f32 / height as f32);
    let resized_width = ((width as f32 * scale).round() as u32).max(1);
    let resized_height = ((height as f32 * scale).round() as u32).max(1);
    let resized = imageops::resize(
        source,
        resized_width,
        resized_height,
        imageops::FilterType::Lanczos3,
    );

    let mut canvas = RgbaImage::from_pixel(size, size, Rgba([0, 0, 0, 0]));
    let x = (size - resized_width) / 2;
    let y = (size - resized_height) / 2;
    imageops::overlay(&mut canvas, &resized, i64::from(x), i64::from(y));
    canvas
}

fn encode_png(image: &RgbaImage, size: u32) -> Result<Vec<u8>, String> {
    let mut output = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|error| format!("failed to encode {size}x{size} PNG: {error}"))?;
    Ok(output.into_inner())
}

fn encode_ico(entries: &[(u32, Vec<u8>)]) -> Result<Vec<u8>, String> {
    if entries.is_empty() || entries.len() > u16::MAX as usize {
        return Err("ICO must contain between 1 and 65535 images".to_string());
    }

    let directory_size = 6usize
        .checked_add(
            entries
                .len()
                .checked_mul(16)
                .ok_or("ICO directory is too large")?,
        )
        .ok_or("ICO directory is too large")?;
    let mut result = Vec::with_capacity(
        directory_size
            + entries
                .iter()
                .map(|(_, payload)| payload.len())
                .sum::<usize>(),
    );
    result.extend_from_slice(&0u16.to_le_bytes());
    result.extend_from_slice(&1u16.to_le_bytes());
    result.extend_from_slice(&(entries.len() as u16).to_le_bytes());

    let mut offset = directory_size as u32;
    for (size, payload) in entries {
        if *size > 256 || *size == 0 || payload.len() > u32::MAX as usize {
            return Err(format!(
                "unsupported ICO entry: {size}px, {} bytes",
                payload.len()
            ));
        }
        result.push(if *size == 256 { 0 } else { *size as u8 });
        result.push(if *size == 256 { 0 } else { *size as u8 });
        result.push(0);
        result.push(0);
        result.extend_from_slice(&1u16.to_le_bytes());
        result.extend_from_slice(&32u16.to_le_bytes());
        result.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        result.extend_from_slice(&offset.to_le_bytes());
        offset = offset
            .checked_add(payload.len() as u32)
            .ok_or("ICO payload is too large")?;
    }
    for (_, payload) in entries {
        result.extend_from_slice(payload);
    }
    Ok(result)
}

fn write_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    fs::write(path, contents)
        .map_err(|error| format!("failed to write '{}': {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_png_variants_and_real_ico_entries() {
        let root = tempfile::tempdir().expect("temp directory");
        let input = root.path().join("source.png");
        let source = RgbaImage::from_fn(8, 4, |x, y| {
            Rgba([(x * 20) as u8, (y * 40) as u8, 200, 255])
        });
        source.save(&input).expect("source PNG");

        let generated = generate_icons(&input, root.path().join("icons"), "app").expect("icons");
        assert!(generated.ico.is_file());
        assert_eq!(generated.pngs.len(), ICON_SIZES.len());

        let ico = fs::read(generated.ico).expect("ICO");
        assert_eq!(&ico[0..4], &[0, 0, 1, 0]);
        assert_eq!(
            u16::from_le_bytes([ico[4], ico[5]]),
            ICON_SIZES.len() as u16
        );
        assert_eq!(ico[6], 16);
        assert_eq!(ico[7], 16);
        let first_payload = u32::from_le_bytes([ico[18], ico[19], ico[20], ico[21]]) as usize;
        assert_eq!(&ico[first_payload..first_payload + 8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn rejects_path_traversal_names() {
        let root = tempfile::tempdir().expect("temp directory");
        let input = root.path().join("source.png");
        RgbaImage::new(1, 1).save(&input).expect("source PNG");
        assert!(generate_icons(&input, root.path(), "../app").is_err());
    }
}
