pub mod image;
pub mod typst_block;

use crate::term::TermCaps;

/// Render a specific page range of the Markdown document (1-based, inclusive).
pub fn render_document_pages(
    markdown: &[u8],
    caps: &TermCaps,
    start: usize,
    end: usize,
) -> Vec<Vec<u8>> {
    typst_block::render_pages(markdown, caps, true, Some((start, end))).unwrap_or_default()
}