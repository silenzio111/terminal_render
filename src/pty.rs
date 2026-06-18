//! PTY management — spawn child process in a pseudo-terminal,
//! forward stdin, and render Markdown output.
use anyhow::Context;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use crate::parser::Parser;
use crate::render;
use crate::term;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PagerMode {
    /// Output all pages and exit; do not enter the pager or wait for keys.
    Disabled,
    /// Default: output all pages, then wait for a key before optionally entering the pager.
    OnDemand,
    /// Enter the interactive pager immediately.
    Immediate,
}

fn get_terminal_size() -> PtySize {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0 {
        PtySize {
            rows: ws.ws_row,
            cols: ws.ws_col,
            pixel_width: ws.ws_xpixel as u16,
            pixel_height: ws.ws_ypixel as u16,
        }
    } else {
        PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

pub fn run(command: &[String], batch: bool, pager_mode: PagerMode) -> anyhow::Result<()> {
    let mut sigset = nix::sys::signal::SigSet::empty();
    sigset.add(nix::sys::signal::Signal::SIGWINCH);
    sigset.add(nix::sys::signal::Signal::SIGINT);
    sigset.add(nix::sys::signal::Signal::SIGTERM);
    nix::sys::signal::sigprocmask(nix::sys::signal::SigmaskHow::SIG_BLOCK, Some(&sigset), None)
        .context("failed to block signals")?;

    let caps = Arc::new(Mutex::new(term::detect()));
    let initial_caps = caps.lock().expect("term caps mutex poisoned").clone();
    eprintln!(
        "mdx: kitty_graphics={}, true_color={}, cols={}, rows={}, cell={}x{}, fg={:?}, bg={:?}",
        initial_caps.kitty_graphics,
        initial_caps.true_color,
        initial_caps.cols,
        initial_caps.rows,
        initial_caps.cell_width,
        initial_caps.cell_height,
        initial_caps.fg_rgb,
        initial_caps.bg_rgb
    );

    let pty_system = NativePtySystem::default();
    let pty_size = get_terminal_size();

    let pair = pty_system.openpty(pty_size).context("failed to open PTY")?;

    let cmd_name = &command[0];
    let mut cmd = CommandBuilder::new(cmd_name);
    if command.len() > 1 {
        cmd.args(&command[1..]);
    }
    cmd.env("TERM", "xterm-256color");
    // Inherit the parent's working directory
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }
    if let Ok(ct) = std::env::var("COLORTERM") {
        cmd.env("COLORTERM", ct);
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .context("failed to spawn command")?;
    let mut child_killer = child.clone_killer();
    drop(pair.slave);

    let master = pair.master;
    let mut master_writer = master.take_writer().context("failed to take PTY writer")?;
    let master_reader = master
        .try_clone_reader()
        .context("failed to clone PTY reader")?;

    // Used to signal threads to stop
    let running = Arc::new(AtomicBool::new(true));

    // ── Signal handling thread ──
    std::thread::spawn({
        let running = running.clone();
        let caps = caps.clone();
        let sigset = sigset;
        move || {
            while running.load(Ordering::Relaxed) {
                match sigset.wait() {
                    Ok(nix::sys::signal::Signal::SIGWINCH) => {
                        let new_size = get_terminal_size();
                        let _ = master.resize(new_size);
                        if let Ok(mut caps) = caps.lock() {
                            term::update_size(&mut caps);
                        }
                    }
                    Ok(nix::sys::signal::Signal::SIGINT)
                    | Ok(nix::sys::signal::Signal::SIGTERM) => {
                        running.store(false, Ordering::Relaxed);
                        let _ = child_killer.kill();
                        break;
                    }
                    _ => break,
                }
            }
        }
    });

    // ── stdin → PTY writer thread ──
    // When an interactive pager is used the pager itself reads from stdin, so
    // we must not forward stdin to the child PTY.  Dropping master_writer here
    // signals EOF on the child's stdin.  Batch commands like `cat file.md` do
    // not need stdin; users who require stdin forwarding should use streaming
    // mode (no file arguments, no --batch) or --no-pager.
    if pager_mode == PagerMode::Immediate || pager_mode == PagerMode::OnDemand {
        drop(master_writer);
    } else {
        std::thread::spawn({
            let running = running.clone();
            move || {
                use std::io::Read;
                let mut stdin = std::io::stdin();
                let mut buf = [0u8; 4096];
                while running.load(Ordering::Relaxed) {
                    match stdin.read(&mut buf) {
                        Ok(0) => break, // EOF on stdin — stop writing but don't kill PTY
                        Ok(n) => {
                            if let Err(e) = master_writer.write_all(&buf[..n]) {
                                // EIO is expected once the child has closed the PTY.
                                if e.raw_os_error() != Some(libc::EIO) {
                                    eprintln!("mdx: write to PTY failed: {}", e);
                                }
                                break;
                            }
                            let _ = master_writer.flush();
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
                // Drop master_writer to signal EOF to child
                drop(master_writer);
            }
        });
    }

    if batch {
        run_batch(master_reader, caps, pager_mode)
    } else {
        run_stream(master_reader, caps)
    }
    .and_then(|_| {
        let _ = child.wait();
        Ok(())
    })
}

/// Read the complete PTY output and render it as pages.
fn run_batch(
    master_reader: Box<dyn std::io::Read + Send>,
    caps: Arc<Mutex<term::TermCaps>>,
    pager_mode: PagerMode,
) -> anyhow::Result<()> {
    let mut output_buf = Vec::new();
    {
        use std::io::Read;
        let mut reader = master_reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // child exited
                Ok(n) => output_buf.extend_from_slice(&buf[..n]),
                Err(e) => {
                    eprintln!("mdx: PTY read error: {}", e);
                    break;
                }
            }
        }
    }

    eprintln!("mdx: captured {} bytes from child", output_buf.len());

    if !output_buf.is_empty() {
        let caps_snapshot = caps.lock().expect("term caps mutex poisoned").clone();
        match pager_mode {
            PagerMode::Immediate => {
                let _ = crate::pager::run(&output_buf, &caps_snapshot, None)?;
            }
            PagerMode::OnDemand => {
                crate::pager::normal_then_interactive(&output_buf, &caps_snapshot)?;
            }
            PagerMode::Disabled => {
                let rendered = render::render_document(&output_buf, &caps_snapshot);
                eprintln!("mdx: rendered {} bytes", rendered.len());
                let mut stdout = std::io::stdout();
                let _ = stdout.write_all(&rendered);
                let _ = stdout.flush();
            }
        }
    }

    Ok(())
}

/// Stream PTY output, rendering each Markdown block as it is completed.
fn run_stream(
    master_reader: Box<dyn std::io::Read + Send>,
    caps: Arc<Mutex<term::TermCaps>>,
) -> anyhow::Result<()> {
    let running = Arc::new(AtomicBool::new(true));

    // ── PTY reader → channel thread ──
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let running_reader = running.clone();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut reader = master_reader;
        let mut buf = [0u8; 4096];
        while running_reader.load(Ordering::Relaxed) {
            match reader.read(&mut buf) {
                Ok(0) => break, // child exited
                Ok(n) => {
                    let _ = tx.send(buf[..n].to_vec());
                }
                Err(e) => {
                    eprintln!("mdx: PTY read error: {}", e);
                    break;
                }
            }
        }
        running_reader.store(false, Ordering::Relaxed);
    });

    // ── Main loop: receive → parse → render → stdout ──
    let mut parser = Parser::new();
    let mut stdout = std::io::stdout();
    let mut last_data_at = std::time::Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(data) => {
                last_data_at = std::time::Instant::now();
                for &b in &data {
                    if let Some(cmd) = parser.feed(b) {
                        let caps_snapshot = caps.lock().expect("term caps mutex poisoned").clone();
                        let rendered = render::render(cmd, &caps_snapshot);
                        let _ = stdout.write_all(&rendered);
                    }
                }
                let _ = stdout.flush();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_data_at.elapsed() >= Duration::from_millis(120) {
                    if let Some(cmd) = parser.drain_pending() {
                        let caps_snapshot = caps.lock().expect("term caps mutex poisoned").clone();
                        let rendered = render::render(cmd, &caps_snapshot);
                        let _ = stdout.write_all(&rendered);
                        let _ = stdout.flush();
                    }
                }

                if !running.load(Ordering::Relaxed) {
                    while let Ok(data) = rx.try_recv() {
                        for &b in &data {
                            if let Some(cmd) = parser.feed(b) {
                                let caps_snapshot =
                                    caps.lock().expect("term caps mutex poisoned").clone();
                                let rendered = render::render(cmd, &caps_snapshot);
                                let _ = stdout.write_all(&rendered);
                            }
                        }
                        if let Some(cmd) = parser.drain_pending() {
                            let caps_snapshot =
                                caps.lock().expect("term caps mutex poisoned").clone();
                            let rendered = render::render(cmd, &caps_snapshot);
                            let _ = stdout.write_all(&rendered);
                        }
                        let _ = stdout.flush();
                    }
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if let Some(cmd) = parser.flush() {
        let caps_snapshot = caps.lock().expect("term caps mutex poisoned").clone();
        let rendered = render::render(cmd, &caps_snapshot);
        let _ = stdout.write_all(&rendered);
        let _ = stdout.flush();
    }

    Ok(())
}
