use crate::render::image;
use crate::term::TermCaps;
use anyhow::Context;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn render(markdown: &[u8], caps: &TermCaps, paginate: bool) -> Option<Vec<Vec<u8>>> {
    render_pages(markdown, caps, paginate, None)
}

/// Render a specific range of pages (1-based, inclusive).
/// `None` means render all pages.
pub fn render_pages(
    markdown: &[u8],
    caps: &TermCaps,
    paginate: bool,
    page_range: Option<(usize, usize)>,
) -> Option<Vec<Vec<u8>>> {
    if !caps.kitty_graphics {
        return None;
    }

    let typst_body = match markdown_to_typst(markdown) {
        Ok(body) => body,
        Err(e) => {
            eprintln!("mdx: pandoc conversion failed: {}", e);
            return None;
        }
    };
    let page_height_pt = if paginate {
        Some(terminal_height_pt(caps))
    } else {
        None
    };
    let typst_source = wrap_typst(&typst_body, caps, page_height_pt);
    if page_range.is_none() {
        eprintln!("mdx: typst source length = {} bytes", typst_source.len());
    }
    let ppi = typst_ppi();
    let pngs = match compile_typst_png(&typst_source, ppi, paginate, page_range) {
        Ok(pngs) => pngs,
        Err(e) => {
            eprintln!("mdx: typst compile failed: {}", e);
            return None;
        }
    };
    if page_range.is_none() {
        eprintln!("mdx: generated {} png page(s)", pngs.len());
    } else {
        eprintln!(
            "mdx: rendered pages {:?}: {} png(s)",
            page_range,
            pngs.len()
        );
    }
    for (i, png) in pngs.iter().enumerate() {
        eprintln!(
            "mdx: page {} size = {} bytes, dimensions = {:?}",
            i + 1,
            png.len(),
            image::png_dimensions(png)
        );
    }
    Some(
        pngs.into_iter()
            .map(|png| image::render_png_block_native(&png, caps))
            .collect(),
    )
}

fn markdown_to_typst(markdown: &[u8]) -> anyhow::Result<String> {
    let mut child = Command::new("pandoc")
        .args([
            "-f",
            "markdown+tex_math_dollars+pipe_tables+raw_html+table_captions",
            "-t",
            "typst",
            "--wrap=none",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(markdown)?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        anyhow::bail!(
            "pandoc failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?)
}

fn compile_typst_png(
    source: &str,
    ppi: u32,
    multi_page: bool,
    page_range: Option<(usize, usize)>,
) -> anyhow::Result<Vec<Vec<u8>>> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;

    // Put the source file in the current working directory so that relative
    // image paths in the Markdown/Typst source resolve correctly.
    let source_file = tempfile::Builder::new()
        .prefix(".mdx-block-")
        .suffix(".typ")
        .tempfile_in(&cwd)
        .context("failed to create temp source file")?;
    let typ_path = source_file.path().to_path_buf();
    std::fs::write(&typ_path, source)?;

    // Generated PNGs go into a temp subdirectory.
    let out_dir = tempfile::TempDir::new_in(&cwd).context("failed to create temp output dir")?;
    let png_arg: std::path::PathBuf = if multi_page {
        out_dir.path().join("block-{n}.png")
    } else {
        out_dir.path().join("block.png")
    };

    let mut command = Command::new("typst");
    command
        .arg("compile")
        // Source is in cwd, so relative image paths resolve from cwd.
        // Root is widened so files outside cwd (e.g. ../output/figures/...)
        // can still be read by local documents.
        .arg("--root")
        .arg("/")
        .arg("--format")
        .arg("png")
        .arg("--ppi")
        .arg(ppi.to_string());

    if let Some((start, end)) = page_range {
        command.arg("--pages").arg(format!("{}-{}", start, end));
    }

    let font_dir = font_path();
    if font_dir.exists() {
        command.arg("--font-path").arg(&font_dir);
    }

    let output = command
        .arg(&typ_path)
        .arg(&png_arg)
        .stderr(Stdio::piped())
        .output()?;

    // The temp source file is deleted when `source_file` is dropped.
    // Make sure it lives until after typst has run.
    drop(source_file);

    if !output.status.success() {
        anyhow::bail!(
            "typst failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Collect all generated PNG files, sorted by filename.
    let mut png_files: Vec<std::path::PathBuf> = std::fs::read_dir(out_dir.path())?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .collect();
    png_files.sort();

    png_files
        .into_iter()
        .map(|path| std::fs::read(path).map_err(|e| e.into()))
        .collect()
}

fn wrap_typst(body: &str, caps: &TermCaps, page_height_pt: Option<f32>) -> String {
    let width_pt = terminal_width_pt(caps);
    let height_clause = match page_height_pt {
        Some(h) => format!("height: {h:.2}pt"),
        None => "height: auto".to_string(),
    };
    let font_size = font_size_pt(caps);
    let table_font_size = (font_size * 0.9).max(10.0);
    let raw_font_size = (font_size * 0.92).max(10.0);
    // Force white background with black text for a conventional paper look.
    let fg = "000000";
    let bg = "ffffff";

    format!(
        r#"#set page(width: {width_pt:.2}pt, {height_clause}, margin: (x: 10pt, y: 10pt), fill: rgb("{bg}"))
#set text(fill: rgb("{fg}"), size: {font_size:.2}pt, font: ("Noto Sans CJK SC", "Noto Sans", "LXGW WenKai"))
#set par(leading: 0.78em, justify: false)
#show table: set text(size: {table_font_size:.2}pt)
#show raw: set text(font: ("JetBrains Mono", "Fira Code", "Menlo"), size: {raw_font_size:.2}pt)
#set table(stroke: rgb("{fg}") + 0.7pt, inset: 6pt)

#block(width: 100%, inset: (x: 4pt, y: 4pt))[
{body}
]
"#
    )
}

fn terminal_width_pt(caps: &TermCaps) -> f32 {
    terminal_pixel_width(caps) as f32 * 0.75
}

fn font_path() -> std::path::PathBuf {
    std::env::var("MDX_FONT_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("fonts"))
}

fn terminal_height_pt(caps: &TermCaps) -> f32 {
    terminal_pixel_height(caps) as f32 * 0.75
}

fn font_size_pt(caps: &TermCaps) -> f32 {
    std::env::var("MDX_TYPST_FONT_PT")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| (8.0..=28.0).contains(v))
        .unwrap_or_else(|| (caps.cell_height.max(1) as f32 * 0.94).clamp(11.0, 20.0))
}

fn typst_ppi() -> u32 {
    std::env::var("MDX_TYPST_PPI")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| (72..=600).contains(v))
        .unwrap_or(200)
}

fn terminal_pixel_width(caps: &TermCaps) -> u32 {
    if caps.pixel_width > 0 {
        caps.pixel_width as u32
    } else {
        caps.cols as u32 * caps.cell_width.max(1) as u32
    }
    .max(320)
}

fn terminal_pixel_height(caps: &TermCaps) -> u32 {
    if caps.pixel_height > 0 {
        caps.pixel_height as u32
    } else {
        caps.rows as u32 * caps.cell_height.max(1) as u32
    }
    .max(240)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::TermCaps;

    fn caps() -> TermCaps {
        TermCaps {
            kitty_graphics: true,
            true_color: true,
            cols: 80,
            rows: 24,
            pixel_width: 640,
            pixel_height: 384,
            cell_width: 8,
            cell_height: 16,
            fg_rgb: None,
            bg_rgb: None,
        }
    }

    #[test]
    fn wraps_page_width() {
        let source = wrap_typst("hello", &caps(), None);
        assert!(source.contains("#set page"));
        assert!(source.contains("hello"));
    }

    #[test]
    fn supports_fixed_page_height() {
        let source = wrap_typst("hello", &caps(), Some(300.0));
        assert!(source.contains("height: 300.00pt"));
    }
}
