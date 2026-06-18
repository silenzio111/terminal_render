pub mod image;
pub mod typst_block;

use crate::parser::RenderCmd;
use crate::term::TermCaps;

/// Render the complete Markdown document as one or more paginated images.
pub fn render_document(markdown: &[u8], caps: &TermCaps) -> Vec<u8> {
    render_document_as_pages(markdown, caps)
        .into_iter()
        .flatten()
        .collect()
}

/// Render the complete Markdown document, returning each page separately.
pub fn render_document_as_pages(markdown: &[u8], caps: &TermCaps) -> Vec<Vec<u8>> {
    typst_block::render(markdown, caps, true).unwrap_or_default()
}

/// Render a specific page range of the Markdown document (1-based, inclusive).
pub fn render_document_pages(
    markdown: &[u8],
    caps: &TermCaps,
    start: usize,
    end: usize,
) -> Vec<Vec<u8>> {
    typst_block::render_pages(markdown, caps, true, Some((start, end))).unwrap_or_default()
}

/// Render one parsed Markdown block.
pub fn render(cmd: RenderCmd, caps: &TermCaps) -> Vec<u8> {
    match cmd {
        RenderCmd::MarkdownBlock(data) => typst_block::render(&data, caps, false)
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect(),
    }
}
