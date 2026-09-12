/// Detect terminal capabilities (Kitty graphics protocol, true color, etc.)
use std::io::{self, Read, Write};
use std::time::Duration;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct TermCaps {
    pub kitty_graphics: bool,
    pub true_color: bool,
    pub cols: u16,
    pub rows: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
    pub cell_width: u16,
    pub cell_height: u16,
    pub fg_rgb: Option<(u8, u8, u8)>,
    pub bg_rgb: Option<(u8, u8, u8)>,
}

#[derive(Clone, Copy, Debug)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

/// Query terminal for Kitty graphics protocol support via DCS escape.
/// Sends: ESC _ Gi=1,s=1,v=1,a=q ESC \
/// If the terminal supports kitty graphics, it replies with the same query.
fn with_raw_terminal<T>(timeout_ds: u8, f: impl FnOnce() -> T) -> Option<T> {
    use std::os::unix::io::RawFd;

    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    let fd: RawFd = libc::STDIN_FILENO;

    if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
        return None;
    }

    let mut raw = termios;
    raw.c_lflag &= !(libc::ECHO | libc::ICANON);
    raw.c_cc[libc::VMIN] = 0;
    raw.c_cc[libc::VTIME] = timeout_ds;

    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        return None;
    }

    let value = f();
    let _ = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) };
    Some(value)
}

/// Query terminal for Kitty graphics protocol support via DCS escape.
/// Sends: ESC _ Gi=1,s=1,v=1,a=q ESC \
/// If the terminal supports kitty graphics, it replies with the same query.
fn probe_kitty() -> bool {
    with_raw_terminal(1, || {
        let query = b"\x1b_Gi=1,s=1,v=1,a=q\x1b\\";
        let mut stdout = io::stdout();
        let _ = stdout.write_all(query);
        let _ = stdout.flush();

        let mut buf = [0u8; 128];
        let mut stdin = io::stdin();
        let mut total = 0;
        let start = std::time::Instant::now();

        while start.elapsed() < Duration::from_secs(1) && total < buf.len() {
            match stdin.read(&mut buf[total..]) {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    if buf[..total].windows(2).any(|w| w == b"\x1b\\") {
                        break;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }

        // Validate: must contain a KGP response (Gi=... inside \x1b_G ... \x1b\\)
        if total > 0 {
            let resp = &buf[..total];
            resp.windows(3).any(|w| w == b"\x1b_G")
                && resp.windows(2).any(|w| w == b"\x1b\\")
                && String::from_utf8_lossy(resp).contains("Gi=")
        } else {
            false
        }
    })
    .unwrap_or(false)
}

/// Check if the terminal is a known KGP-capable emulator via env vars.
fn probe_kitty_by_brand() -> bool {
    let term = std::env::var("TERM").unwrap_or_default();
    let program = std::env::var("TERM_PROGRAM").unwrap_or_default();

    if ["xterm-kitty", "xterm-ghostty", "rio"].contains(&term.as_str()) {
        return true;
    }
    if ["kitty", "ghostty", "rio", "WezTerm", "otty"].contains(&program.as_str()) {
        return true;
    }
    // Otty also sets TERM_PROGRAM to "otty"
    false
}

/// Detect KGP support with probe + brand fallback + override.
fn detect_kitty_graphics() -> bool {
    // Manual override: MDX_FORCE_KITTY=1 or MDX_FORCE_KITTY=0
    if let Ok(val) = std::env::var("MDX_FORCE_KITTY") {
        return val != "0" && val != "false";
    }

    // Brand check first (fast, no I/O)
    if probe_kitty_by_brand() {
        return true;
    }

    // Fall back to active probe
    probe_kitty()
}

/// Get terminal size
pub fn get_size() -> TermSize {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0 {
        TermSize {
            cols: ws.ws_col.max(1),
            rows: ws.ws_row.max(1),
            pixel_width: ws.ws_xpixel,
            pixel_height: ws.ws_ypixel,
        }
    } else {
        TermSize {
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

fn parse_rgb_color(value: &str) -> Option<(u8, u8, u8)> {
    let s = value.strip_prefix("rgb:")?;
    let mut parts = s.split('/');
    let parse = |p: &str| u8::from_str_radix(p.get(0..2)?, 16).ok();
    Some((
        parse(parts.next()?)?,
        parse(parts.next()?)?,
        parse(parts.next()?)?,
    ))
}

fn query_osc_color(selector: u8) -> Option<(u8, u8, u8)> {
    with_raw_terminal(1, || {
        let query = format!("\x1b]{};?\x07", selector);
        let mut stdout = io::stdout();
        let _ = stdout.write_all(query.as_bytes());
        let _ = stdout.flush();

        let mut buf = [0u8; 128];
        let mut stdin = io::stdin();
        let mut total = 0;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_millis(200) && total < buf.len() {
            match stdin.read(&mut buf[total..]) {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    if buf[..total].contains(&b'\x07')
                        || buf[..total].windows(2).any(|w| w == b"\x1b\\")
                    {
                        break;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }

        let response = String::from_utf8_lossy(&buf[..total]);
        response
            .find("rgb:")
            .and_then(|idx| parse_rgb_color(&response[idx..]))
    })
    .flatten()
}

fn color_from_env(name: &str) -> Option<(u8, u8, u8)> {
    std::env::var(name).ok().and_then(|v| parse_rgb_color(&v))
}

fn cell_size(size: TermSize) -> (u16, u16) {
    let cw = if size.pixel_width > 0 && size.cols > 0 {
        (size.pixel_width / size.cols).max(1)
    } else {
        8
    };
    let ch = if size.pixel_height > 0 && size.rows > 0 {
        (size.pixel_height / size.rows).max(1)
    } else {
        16
    };
    (cw, ch)
}

pub fn detect() -> TermCaps {
    // Check COLORTERM for true color support
    let true_color = std::env::var("COLORTERM")
        .map(|v| v == "truecolor" || v == "24bit")
        .unwrap_or(false);

    let size = get_size();
    let (cell_width, cell_height) = cell_size(size);
    let fg_rgb = color_from_env("MDX_FG").or_else(|| query_osc_color(10));
    let bg_rgb = color_from_env("MDX_BG").or_else(|| query_osc_color(11));

    TermCaps {
        kitty_graphics: detect_kitty_graphics(),
        true_color,
        cols: size.cols,
        rows: size.rows,
        pixel_width: size.pixel_width,
        pixel_height: size.pixel_height,
        cell_width,
        cell_height,
        fg_rgb,
        bg_rgb,
    }
}

/// Display a PNG directly at the cursor, scaling it to `cols` columns.
///
/// Rows are omitted so that Kitty computes them automatically from the
/// image's aspect ratio and the terminal's actual cell aspect ratio,
/// preventing the image from being stretched or squashed.
///
/// Payload is chunked at ~20 KiB to stay within terminal APC buffer limits.
pub fn kitty_display_image_escape(png_data: &[u8], image_id: u32, cols: u16) -> Vec<u8> {
    const CHUNK_SIZE: usize = 16384; // 16 KiB of base64 payload per chunk

    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png_data);
    let mut result = Vec::new();

    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK_SIZE).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = if i + 1 < chunks.len() { 1 } else { 0 };
        let header = format!(
            "\x1b_Gf=100,a=T,t=d,q=2,m={},i={},c={};",
            more,
            image_id,
            cols.max(1)
        );
        result.extend_from_slice(header.as_bytes());
        result.extend_from_slice(chunk);
        result.extend_from_slice(b"\x1b\\");
    }

    result
}

/// Delete all images from the terminal's graphics cache.
///
/// Sends `ESC _ Gi=31,a=d,d=A ESC \` which instructs Kitty to purge every
/// stored image.  Use this when navigating between pages or exiting so the
/// cache does not grow without bound.
pub fn kitty_delete_all_images_escape() -> Vec<u8> {
    b"\x1b_Ga=d,d=A\x1b\\".to_vec()
}
