mod pager;
mod render;
mod term;

use clap::Parser as ClapParser;
use std::path::{Path, PathBuf};

#[derive(ClapParser)]
#[command(
    name = "mdx",
    about = "Terminal Markdown renderer — renders .md files as PNG via Typst + Kitty graphics",
)]
struct Args {
    /// Markdown file to render.
    #[arg(required = true)]
    file: PathBuf,

    /// Export the rendered Markdown to a PDF next to the .md file, then exit.
    #[arg(long)]
    pdf: bool,

    /// Use one continuous long page instead of normal A4 pagination.
    #[arg(long, conflicts_with = "paginate", requires = "pdf")]
    continuous: bool,

    /// Use normal A4 pagination (the default for PDF export).
    #[arg(long, conflicts_with = "continuous", requires = "pdf")]
    paginate: bool,
}

/// Verify that external rendering tools are installed and reachable.
fn check_dependencies() -> anyhow::Result<()> {
    for cmd in ["pandoc", "typst"] {
        if std::process::Command::new(cmd)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_err()
        {
            anyhow::bail!(
                "`{}` not found in PATH.\n\
                 mdx requires both `pandoc` and `typst` to render Markdown.\n\n\
                 Install them:\n  \
                 macOS:  brew install pandoc typst\n  \
                 Linux:  sudo apt install pandoc && cargo install typst-cli\n  \
                 Arch:   sudo pacman -S pandoc typst",
                cmd
            );
        }
    }
    Ok(())
}

/// Check whether the bundled fonts directory contains any font files.
/// Returns true if at least one .ttf/.otf/.ttc is found (recursively).
pub fn fonts_dir_has_fonts(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if fonts_dir_has_fonts(&path) {
                return true;
            }
        } else if let Some(ext) = path.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            if ext == "ttf" || ext == "otf" || ext == "ttc" {
                return true;
            }
        }
    }
    false
}

/// Warn the user if the bundled fonts directory is empty.
fn check_fonts() {
    let font_dir = std::env::var("MDX_FONT_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("fonts"));
    if !fonts_dir_has_fonts(&font_dir) {
        eprintln!(
            "mdx: warning: no font files found in `{}`. \
             Rendering will rely on system fonts. \
             See `fonts/README.md` for how to bundle fonts.",
            font_dir.display()
        );
    }
}

fn main() {
    let args = Args::parse();

    if let Err(e) = check_dependencies() {
        eprintln!("mdx: {}", e);
        std::process::exit(1);
    }
    check_fonts();

    let file = &args.file;
    if !file.is_file() {
        eprintln!("mdx: file not found: {}", file.display());
        std::process::exit(1);
    }

    let markdown = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("mdx: failed to read {}: {}", file.display(), e);
            std::process::exit(1);
        }
    };

    if args.pdf {
        let pdf_path = file.with_extension("pdf");
        let work_dir = file.parent().unwrap_or_else(|| Path::new("."));
        let paginate = args.paginate || !args.continuous;
        if let Err(e) = render::typst_block::export_pdf(
            &markdown,
            &pdf_path,
            work_dir,
            paginate,
        ) {
            eprintln!("mdx: failed to write PDF: {}", e);
            std::process::exit(1);
        }
        eprintln!("mdx: wrote {}", pdf_path.display());
        return;
    }

    let caps = term::detect();

    if let Err(e) = pager::run(markdown, caps, Some(file.clone())) {
        eprintln!("mdx: error: {}", e);
        std::process::exit(1);
    }
}
