/// Detect terminal capabilities (Kitty graphics protocol, true color, etc.)
use std::io::{self, Read, Write};
use std::time::Duration;

#[derive(Clone, Debug)]
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
        // ESC _ G i = 1 , s = 1 , v = 1 , a = q ESC \
        let query = b"\x1b_Gi=1,s=1,v=1,a=q\x1b\\";
        let mut stdout = io::stdout();
        let _ = stdout.write_all(query);
        let _ = stdout.flush();

        // Read response with timeout
        let mut buf = [0u8; 64];
        let mut stdin = io::stdin();
        let mut total = 0;
        let start = std::time::Instant::now();

        while start.elapsed() < Duration::from_millis(200) && total < buf.len() {
            match stdin.read(&mut buf[total..]) {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    if buf[..total].windows(2).any(|w| w == b"\x1b\\") {
                        break;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }

        total > 0
    })
    .unwrap_or(false)
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
        kitty_graphics: probe_kitty(),
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

pub fn update_size(caps: &mut TermCaps) {
    let size = get_size();
    let (cell_width, cell_height) = cell_size(size);
    caps.cols = size.cols;
    caps.rows = size.rows;
    caps.pixel_width = size.pixel_width;
    caps.pixel_height = size.pixel_height;
    caps.cell_width = cell_width;
    caps.cell_height = cell_height;
}

/// Display a PNG directly at the cursor, scaling it to `cols` columns.
///
/// Rows are omitted so that Kitty computes them automatically from the
/// image's aspect ratio and the terminal's actual cell aspect ratio,
/// preventing the image from being stretched or squashed.
pub fn kitty_display_image_escape(png_data: &[u8], image_id: u32, cols: u16) -> Vec<u8> {
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png_data);
    let header = format!(
        "\x1b_Gf=100,a=T,t=d,q=2,i={},c={};",
        image_id,
        cols.max(1)
    );

    let mut result = Vec::with_capacity(header.len() + b64.len() + 2);
    result.extend_from_slice(header.as_bytes());
    result.extend_from_slice(b64.as_bytes());
    result.extend_from_slice(b"\x1b\\");
    result
}
