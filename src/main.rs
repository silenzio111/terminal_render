mod pager;
mod parser;
mod pty;
mod render;
mod term;

use clap::Parser as ClapParser;
use std::path::Path;

#[derive(ClapParser)]
#[command(
    name = "mdx",
    about = "Terminal Markdown renderer — a PTY proxy for CLI AI tools",
    long_about = "Wraps any CLI program (Claude Code, Codex, etc.) and renders \
                  Markdown output as PNG images via Typst and the Kitty graphics protocol."
)]
struct Args {
    /// Render the whole output as one image instead of streaming block by block.
    #[arg(long)]
    batch: bool,

    /// Enter the interactive pager immediately (default is normal output).
    #[arg(long)]
    pager: bool,

    /// Output all pages and exit; do not wait for a key or enter the pager.
    #[arg(long)]
    no_pager: bool,

    /// Command to run (e.g., "claude", "codex", "cat README.md")
    #[arg(required = true, num_args = 1..)]
    command: Vec<String>,
}

/// Heuristic: treat the command as batch mode if any argument is an existing file.
/// This catches `mdx cat file.md`, `mdx pandoc ... file.md`, etc.
fn detect_batch(command: &[String]) -> bool {
    command.iter().any(|arg| Path::new(arg).is_file())
}

fn main() {
    let args = Args::parse();
    let batch = args.batch || detect_batch(&args.command);

    // Determine pager behavior for batch mode.
    let pager_mode = if args.no_pager {
        pty::PagerMode::Disabled
    } else if args.pager {
        pty::PagerMode::Immediate
    } else {
        // Default batch behavior: output all pages, then wait for a key.
        pty::PagerMode::OnDemand
    };

    eprintln!("mdx: batch mode = {}, pager mode = {:?}", batch, pager_mode);

    if let Err(e) = pty::run(&args.command, batch, pager_mode) {
        eprintln!("mdx: error: {}", e);
        std::process::exit(1);
    }
}
