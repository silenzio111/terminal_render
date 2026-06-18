/// Stream bytes into markdown blocks.
///
/// This parser is intentionally shallow: it only splits the PTY stream
/// into renderable blocks; Markdown semantics are handled by Typst.

#[derive(Debug, Clone)]
pub enum RenderCmd {
    MarkdownBlock(Vec<u8>),
}

pub struct Parser {
    buf: Vec<u8>,
    newline_run: usize,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            newline_run: 0,
        }
    }

    pub fn feed(&mut self, b: u8) -> Option<RenderCmd> {
        if b == b'\r' {
            return None;
        }
        if b == b'\n' {
            self.newline_run += 1;
        } else {
            self.newline_run = 0;
        }
        self.buf.push(b);

        // Flush on paragraph boundary to keep markdown block parsing stable.
        if self.newline_run >= 2 {
            Some(RenderCmd::MarkdownBlock(std::mem::take(&mut self.buf)))
        } else {
            None
        }
    }

    pub fn drain_pending(&mut self) -> Option<RenderCmd> {
        if self.buf.is_empty() {
            None
        } else {
            Some(RenderCmd::MarkdownBlock(std::mem::take(&mut self.buf)))
        }
    }

    pub fn flush(&mut self) -> Option<RenderCmd> {
        self.drain_pending()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flushes_on_blank_line() {
        let mut p = Parser::new();
        let mut out = Vec::new();
        for b in b"hello\n\nworld".iter().copied() {
            if let Some(cmd) = p.feed(b) {
                out.push(cmd);
            }
        }
        assert!(matches!(out[0], RenderCmd::MarkdownBlock(_)));
    }
}
