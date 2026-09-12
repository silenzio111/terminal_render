//! Interactive pager for batch-rendered Markdown pages.
//!
//! Pages are rendered lazily in batches of 8 so that very long documents do
//! not overwhelm the terminal image cache.  The user can navigate forward,
//! backward, first, and last with single keypresses.
//!
//! When a watch file is provided the pager polls its mtime and re-renders
//! automatically, keeping the reading position proportional to the document.

use crate::render;
use crate::term::{kitty_delete_all_images_escape, TermCaps};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const BATCH_SIZE: usize = 8;
const POLL_MS: i32 = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Next,
    Prev,
    First,
    Last,
    Quit,
    Unknown,
}

struct PagerState {
    markdown: Vec<u8>,
    caps: TermCaps,
    /// Rendered pages.  Sparse: batches are loaded on demand.
    pages: Vec<Option<Vec<u8>>>,
    current: usize,
    /// True once a batch has returned fewer than BATCH_SIZE pages.
    exhausted: bool,
}

impl PagerState {
    fn new(markdown: Vec<u8>, caps: TermCaps) -> Self {
        Self {
            markdown,
            caps,
            pages: Vec::new(),
            current: 0,
            exhausted: false,
        }
    }

    /// Number of pages currently known to exist.
    fn len(&self) -> usize {
        self.pages.len()
    }

    /// True when the total page count is known.
    fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// Reading progress as a ratio in [0, 1].
    fn progress(&self) -> f64 {
        if self.pages.is_empty() {
            0.0
        } else {
            self.current as f64 / (self.pages.len() - 1).max(1) as f64
        }
    }

    /// Ensure the batch containing `index` is loaded.
    fn ensure_loaded(&mut self, index: usize) {
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

        let rendered = render::render_document_pages(&self.markdown, &self.caps, start, end);
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
        if self.current + 1 < self.pages.len() {
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
        self.current = index.min(self.pages.len().saturating_sub(1));
    }

    fn go_to_last(&mut self) {
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

    /// Place the cursor at the page closest to the given progress ratio.
    fn go_to_progress(&mut self, ratio: f64) {
        // Load enough pages to know the total.
        self.go_to_last();
        if self.pages.is_empty() {
            return;
        }
        let target = (ratio * (self.pages.len() - 1) as f64).round() as usize;
        self.current = target.min(self.pages.len() - 1);
    }

    fn current_page(&self) -> Option<&[u8]> {
        self.pages.get(self.current).and_then(|p| p.as_deref())
    }
}

pub fn run(markdown: Vec<u8>, caps: TermCaps, watch_file: Option<PathBuf>) -> anyhow::Result<()> {
    let mut state = PagerState::new(markdown, caps);
    state.ensure_loaded(0);

    // Track the file mtime for live re-rendering.
    let mut watch = watch_file.map(WatchState::new);

    with_raw_terminal(|| {
        let mut stdout = io::stdout();
        display_current(&state, &mut stdout, watch.as_ref())?;

        let stdin = io::stdin();
        let mut stdin_lock = stdin.lock();
        let mut key_buf = Vec::new();
        loop {
            if watch.is_some() {
                // Non-blocking: poll stdin with a timeout, then check mtime.
                if poll_stdin(POLL_MS) {
                    let key = read_key(&mut stdin_lock, &mut key_buf)?;
                    if handle_key(key, &mut state, &mut stdout, watch.as_ref())? {
                        break;
                    }
                } else if let Some(ref mut w) = watch {
                    if w.check_changed() {
                        rerender(&mut state, w, &mut stdout)?;
                    }
                }
            } else {
                let key = read_key(&mut stdin_lock, &mut key_buf)?;
                if handle_key(key, &mut state, &mut stdout, None)? {
                    break;
                }
            }
        }

        // Clear the screen, free images, and reset cursor on exit.
        let _ = stdout.write_all(&kitty_delete_all_images_escape());
        let _ = stdout.write_all(b"\x1b[2J\x1b[H");
        stdout.flush()?;
        Ok(())
    })
}

struct WatchState {
    path: PathBuf,
    last_mtime: SystemTime,
}

impl WatchState {
    fn new(path: PathBuf) -> Self {
        let last_mtime = file_mtime(&path).unwrap_or(SystemTime::UNIX_EPOCH);
        Self { path, last_mtime }
    }

    /// Returns true if the file's mtime has changed since the last check.
    fn check_changed(&mut self) -> bool {
        match file_mtime(&self.path) {
            Some(mtime) if mtime != self.last_mtime => {
                self.last_mtime = mtime;
                true
            }
            _ => false,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
}

fn rerender(
    state: &mut PagerState,
    watch: &mut WatchState,
    stdout: &mut io::Stdout,
) -> io::Result<()> {
    let ratio = state.progress();
    let content = std::fs::read(watch.path()).unwrap_or_default();
    *state = PagerState::new(content, state.caps.clone());
    state.go_to_progress(ratio);
    display_current(state, stdout, Some(watch))?;
    Ok(())
}

/// Handle a key press. Returns true if the pager should quit.
fn handle_key(
    key: Key,
    state: &mut PagerState,
    stdout: &mut io::Stdout,
    watch: Option<&WatchState>,
) -> anyhow::Result<bool> {
    match key {
        Key::Quit => Ok(true),
        Key::Next => {
            if state.next_page() {
                display_current(state, stdout, watch)?;
            }
            Ok(false)
        }
        Key::Prev => {
            if state.prev_page() {
                display_current(state, stdout, watch)?;
            }
            Ok(false)
        }
        Key::First => {
            state.go_to(0);
            display_current(state, stdout, watch)?;
            Ok(false)
        }
        Key::Last => {
            state.go_to_last();
            display_current(state, stdout, watch)?;
            Ok(false)
        }
        Key::Unknown => Ok(false),
    }
}

fn display_current(
    state: &PagerState,
    stdout: &mut io::Stdout,
    watch: Option<&WatchState>,
) -> io::Result<()> {
    // Clear screen and move cursor to top-left.
    stdout.write_all(b"\x1b[2J\x1b[H")?;
    // Free any previously displayed images from the terminal cache so it
    // does not grow unboundedly while navigating.
    stdout.write_all(&kitty_delete_all_images_escape())?;

    let total = if state.is_exhausted() {
        format!("{}", state.len())
    } else {
        "?".to_string()
    };
    let watch_label = if watch.is_some() { " [watch]" } else { "" };
    write!(
        stdout,
        "mdx: page {} / {}  (↓/→: next, ↑/←: prev, g/G: first/last, space/q: quit){}\r\n",
        state.current + 1,
        total,
        watch_label,
    )?;

    if let Some(page) = state.current_page() {
        stdout.write_all(page)?;
    } else {
        stdout.write_all(b"(page not loaded)\r\n")?;
    }
    stdout.flush()
}

/// Poll stdin for data, returning true if data is available within `timeout_ms`.
fn poll_stdin(timeout_ms: i32) -> bool {
    let mut fds = [libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    }];
    let n = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout_ms) };
    n > 0 && (fds[0].revents & libc::POLLIN) != 0
}

fn read_key(stdin: &mut io::StdinLock<'_>, buf: &mut Vec<u8>) -> anyhow::Result<Key> {
    loop {
        if let Some((key, consumed)) = parse_key(buf) {
            buf.drain(..consumed);
            return Ok(key);
        }

        // Need more bytes.
        let mut tmp = [0u8; 8];
        let n = stdin.read(&mut tmp)?;
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
            b' ' | b'q' | b'Q' => Key::Quit,
            b'j' | b'n' | b'\n' | b'\r' => Key::Next,
            b'b' | b'k' | b'p' => Key::Prev,
            b'g' => Key::First,
            b'G' => Key::Last,
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
