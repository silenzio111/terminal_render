//! PNG helpers for the Typst-based rendering pipeline.

use crate::term::{kitty_display_image_escape, TermCaps};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_IMAGE_ID: AtomicU32 = AtomicU32::new(1);

/// Parse width/height from a PNG blob.
pub fn png_dimensions(png_data: &[u8]) -> Option<(u32, u32)> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if png_data.len() < 24 || &png_data[..8] != PNG_SIGNATURE || &png_data[12..16] != b"IHDR" {
        return None;
    }

    Some((
        u32::from_be_bytes(png_data[16..20].try_into().ok()?),
        u32::from_be_bytes(png_data[20..24].try_into().ok()?),
    ))
}

/// Display a PNG as a Kitty graphics image block sized to the terminal.
///
/// The image is scaled to fit within the terminal width. Only the column
/// count is sent to Kitty; the terminal derives the row count from the
/// image's aspect ratio and the real cell aspect ratio, avoiding stretch.
pub fn render_png_block_native(png: &[u8], caps: &TermCaps) -> Vec<u8> {
    let Some((img_w, _img_h)) = png_dimensions(png) else {
        return Vec::new();
    };
    let cell_w = caps.cell_width.max(1) as u32;
    let natural_cols = img_w.div_ceil(cell_w).max(1);
    let cols = natural_cols.min(caps.cols as u32) as u16;

    let image_id = NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed);
    let kitty_esc = kitty_display_image_escape(png, image_id, cols);

    let mut out = Vec::new();
    out.extend_from_slice(&kitty_esc);
    out.push(b'\n');
    out
}
