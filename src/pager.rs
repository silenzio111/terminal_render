//! Interactive pager for batch-rendered Markdown pages.
//!
//! Pages are rendered lazily in batches of 8 so that very long documents do
//! not overwhelm the terminal image cache.  The user can navigate forward,
//! backward, first, and last with single keypresses.

use crate::render;
use crate::term::TermCaps;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

const BATCH_SIZE: usize = 8;

fn debug_log(msg: &str) {
    use std::fs::OpenOptions;
    static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);
    let mut guard = LOG.lock().unwrap();
    if guard.is_none() {
        *guard = OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/mdx_debug.log")
            .ok();
    }
    if let Some(f) = guard.as_mut() {
        let _ = writeln!(f, "{}", msg);
        let _ = f.flush();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Next,
    Prev,
    First,
    Last,
    Quit,
    ExitPager, // space: leave the pager and return to normal mode
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PagerOutcome {
    ExitProgram,
    ExitPager,
}

struct PagerState<'a> {
    markdown: &'a [u8],
    caps: &'a TermCaps,
    /// Rendered pages.  Sparse: batches are loaded on demand.
    pages: Vec<Option<Vec<u8>>>,
    current: usize,
    /// True once a batch has returned fewer than BATCH_SIZE pages.
    exhausted: bool,
    /// Pre-rendered pages shared from normal mode, if available.
    pre_rendered: Option<Arc<Vec<Vec<u8>>>>,
}

impl<'a> PagerState<'a> {
    fn new(
        markdown: &'a [u8],
        caps: &'a TermCaps,
        pre_rendered: Option<Arc<Vec<Vec<u8>>>>,
    ) -> Self {
        Self {
            markdown,
            caps,
            pages: Vec::new(),
            current: 0,
            exhausted: false,
            pre_rendered,
        }
    }

    /// Number of pages currently known to exist.
    fn len(&self) -> usize {
        self.pre_rendered
            .as_ref()
            .map(|p| p.len())
            .unwrap_or_else(|| self.pages.len())
    }

    /// True when the total page count is known.
    fn is_exhausted(&self) -> bool {
        self.pre_rendered.is_some() || self.exhausted
    }

    /// Ensure the batch containing `index` is loaded.
    fn ensure_loaded(&mut self, index: usize) {
        if self.pre_rendered.is_some() {
            return;
        }
        if self.exhausted && index >= self.pages.len() {
            return;
        }

        let batch = index / BATCH_SIZE;
        let start = batch * BATCH_SIZE + 1; // Typst pages are 1-based
        let end = start + BATCH_SIZE - 1;

        // Already loaded if the first slot of this batch is Some.
        if self.pages.get(start - 1).is_some_and(|p| p.is_some()) {
            return;
        }

        debug_log(&format!("loading pages {}-{}", start, end));
        let rendered = render::render_document_pages(self.markdown, self.caps, start, end);
        debug_log(&format!("loaded {} pages", rendered.len()));
        let count = rendered.len();

        // Grow pages vector to fit this batch.
        let required = start - 1 + count.max(BATCH_SIZE);
        if self.pages.len() < required {
            self.pages.resize(required, None);
        }

        for (i, page) in rendered.into_iter().enumerate() {
            self.pages[start - 1 + i] = Some(page);
        }

        if count < BATCH_SIZE {
            self.exhausted = true;
            // Trim trailing empty slots that were pre-allocated.
            while self.pages.last().is_some_and(|p| p.is_none()) {
                self.pages.pop();
            }
        }
    }

    fn next_page(&mut self) -> bool {
        self.ensure_loaded(self.current + 1);
        if self.current + 1 < self.len() {
            self.current += 1;
            true
        } else {
            false
        }
    }

    fn prev_page(&mut self) -> bool {
        if self.current > 0 {
            self.current -= 1;
            true
        } else {
            false
        }
    }

    fn go_to(&mut self, index: usize) {
        self.ensure_loaded(index);
        self.current = index.min(self.len().saturating_sub(1));
    }

    fn go_to_last(&mut self) {
        if let Some(pre) = &self.pre_rendered {
            if !pre.is_empty() {
                self.current = pre.len() - 1;
            }
            return;
        }
        // Load batches until exhausted.
        let mut probe = self.pages.len();
        while !self.exhausted {
            self.ensure_loaded(probe);
            probe += BATCH_SIZE;
        }
        if !self.pages.is_empty() {
            self.current = self.pages.len() - 1;
        }
    }

    fn current_page(&self) -> Option<&[u8]> {
        if let Some(pre) = &self.pre_rendered {
            return pre.get(self.current).map(|p| p.as_slice());
        }
        self.pages.get(self.current).and_then(|p| p.as_deref())
    }
}

pub fn run(
    markdown: &[u8],
    caps: &TermCaps,
    pre_rendered: Option<Vec<Vec<u8>>>,
) -> anyhow::Result<PagerOutcome> {
    debug_log("pager::run started");
    let pre_rendered = pre_rendered.map(Arc::new);
    let mut state = PagerState::new(markdown, caps, pre_rendered);
    state.ensure_loaded(0);
    debug_log(&format!("pager initial page loaded, total known pages {}", state.len()));

    with_raw_terminal(|| {
        debug_log("pager entered raw terminal");
        let mut stdout = io::stdout();
        display_current(&state, &mut stdout)?;
        debug_log("pager displayed current page");

        let stdin = io::stdin();
        let mut stdin_lock = stdin.lock();
        let mut key_buf = Vec::new();
        let outcome = loop {
            let key = read_key(&mut stdin_lock, &mut key_buf)?;
            match key {
                Key::Quit => break PagerOutcome::ExitProgram,
                Key::ExitPager => break PagerOutcome::ExitPager,
                Key::Next => {
                    if state.next_page() {
                        display_current(&state, &mut stdout)?;
                    }
                }
                Key::Prev => {
                    if state.prev_page() {
                        display_current(&state, &mut stdout)?;
                    }
                }
                Key::First => {
                    state.go_to(0);
                    display_current(&state, &mut stdout)?;
                }
                Key::Last => {
                    state.go_to_last();
                    display_current(&state, &mut stdout)?;
                }
                Key::Unknown => {}
            }
        };

        // Clear the screen and reset cursor on exit.
        let _ = stdout.write_all(b"\x1b[2J\x1b[H");
        stdout.flush()?;
        Ok(outcome)
    })
}

/// Default mode: render the whole document, print it, and wait for a key.
/// Arrow keys enter the pager; Space or q exit the program.
/// When the pager is left with Space, the normal view is restored.
pub fn normal_then_interactive(markdown: &[u8], caps: &TermCaps) -> anyhow::Result<()> {
    debug_log(&format!("normal_then_interactive started, markdown {} bytes", markdown.len()));
    let pages = render::render_document_as_pages(markdown, caps);
    debug_log(&format!("rendered {} pages", pages.len()));
    let mut stdout = io::stdout();
    for page in &pages {
        stdout.write_all(page)?;
    }
    stdout.flush()?;

    with_raw_terminal(|| {
        let stdin = io::stdin();
        let mut stdin_lock = stdin.lock();
        let mut key_buf = Vec::new();
        let mut stdout = io::stdout();

        writeln!(
            stdout,
            "\r\nmdx: ↑↓←→ = pager, space/q = quit"
        )?;
        stdout.flush()?;

        loop {
            let key = read_key(&mut stdin_lock, &mut key_buf)?;
            match key {
                Key::Next | Key::Prev | Key::First | Key::Last => {
                    // Release stdin lock before calling run(), otherwise run()
                    // will deadlock waiting for the same StdinLock.
                    drop(stdin_lock);
                    let outcome = run(markdown, caps, Some(pages.clone()))?;
                    stdin_lock = stdin.lock();
                    match outcome {
                        PagerOutcome::ExitPager => {
                            // Restore the normal (continuous) view.
                            stdout.write_all(b"\x1b[2J\x1b[H")?;
                            for page in &pages {
                                stdout.write_all(page)?;
                            }
                            writeln!(
                                stdout,
                                "\r\nmdx: ↑↓←→ = pager, space/q = quit"
                            )?;
                            stdout.flush()?;
                        }
                        PagerOutcome::ExitProgram => break,
                    }
                }
                Key::Quit | Key::ExitPager => break,
                _ => {}
            }
        }

        let _ = stdout.write_all(b"\x1b[2J\x1b[H");
        stdout.flush()?;
        Ok(())
    })
}

fn display_current(state: &PagerState, stdout: &mut io::Stdout) -> io::Result<()> {
    // Clear screen and move cursor to top-left.
    stdout.write_all(b"\x1b[2J\x1b[H")?;

    let total = if state.is_exhausted() {
        format!("{}", state.len())
    } else {
        "?".to_string()
    };
    write!(
        stdout,
        "mdx: page {} / {}  (↓/→: next, ↑/←: prev, g/G: first/last, space: exit pager, q: quit)\r\n",
        state.current + 1,
        total
    )?;

    if let Some(page) = state.current_page() {
        stdout.write_all(page)?;
    } else {
        stdout.write_all(b"(page not loaded)\r\n")?;
    }
    stdout.flush()
}

fn read_key(stdin: &mut io::StdinLock<'_>, buf: &mut Vec<u8>) -> anyhow::Result<Key> {
    loop {
        if let Some((key, consumed)) = parse_key(buf) {
            debug_log(&format!("key parsed {:?}, consumed {}", key, consumed));
            buf.drain(..consumed);
            return Ok(key);
        }

        // Need more bytes.
        let mut tmp = [0u8; 8];
        let n = stdin.read(&mut tmp)?;
        debug_log(&format!("read {} bytes: {:?}", n, &tmp[..n.min(tmp.len())]));
        if n == 0 {
            return Ok(Key::Quit);
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

/// Try to parse the first complete key sequence from the buffer.
/// Returns the key and the number of bytes consumed, or None if incomplete.
fn parse_key(buf: &[u8]) -> Option<(Key, usize)> {
    if buf.is_empty() {
        return None;
    }

    let first = buf[0];
    if first != 0x1b {
        let key = match first {
            b' ' => Key::ExitPager,
            b'j' | b'n' | b'\n' | b'\r' => Key::Next,
            b'b' | b'k' | b'p' => Key::Prev,
            b'g' => Key::First,
            b'G' => Key::Last,
            b'q' | b'Q' => Key::Quit,
            _ => Key::Unknown,
        };
        return Some((key, 1));
    }

    // Escape sequence.  Need at least 3 bytes for CSI sequences.
    if buf.len() < 2 {
        return None;
    }

    // SS3 sequences (e.g. ESC O A for arrow keys in application mode).
    if buf[1] == b'O' && buf.len() >= 3 {
        let key = match buf[2] {
            b'A' => Key::Prev, // up
            b'B' => Key::Next, // down
            b'C' => Key::Next, // right
            b'D' => Key::Prev, // left
            b'H' => Key::First, // home
            b'F' => Key::Last, // end
            _ => Key::Unknown,
        };
        return Some((key, 3));
    }

    // CSI sequences (ESC [ ...).  Read until final byte (0x40-0x7E).
    if buf[1] == b'[' {
        for (i, &b) in buf.iter().enumerate().skip(2) {
            if (0x40..=0x7E).contains(&b) {
                // Look at the final byte; parameterized forms like
                // ESC [ 1 ; 2 A still end in the same character.
                let key = match b {
                    b'A' => Key::Prev,    // up
                    b'B' => Key::Next,    // down
                    b'C' => Key::Next,    // right
                    b'D' => Key::Prev,    // left
                    b'H' => Key::First,   // home
                    b'F' => Key::Last,    // end
                    b'~' => {
                        // Home/End as ESC [ 1 ~ / ESC [ 4 ~
                        if buf.ends_with(b"\x1b[1~") {
                            Key::First
                        } else if buf.ends_with(b"\x1b[4~") {
                            Key::Last
                        } else {
                            Key::Unknown
                        }
                    }
                    _ => Key::Unknown,
                };
                return Some((key, i + 1));
            }
        }
        return None; // incomplete CSI
    }

    // Unrecognized escape sequence: consume ESC + next byte to avoid getting stuck.
    Some((Key::Unknown, 2.min(buf.len())))
}

fn with_raw_terminal<T>(f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut termios) } != 0 {
        anyhow::bail!("failed to get terminal attributes");
    }

    let mut raw = termios;
    raw.c_lflag &= !(libc::ECHO | libc::ICANON);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;

    if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
        anyhow::bail!("failed to set raw terminal mode");
    }

    let result = f();
    let _ = unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &termios) };
    result
}
