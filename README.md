# mdx

Terminal Markdown renderer — renders `.md` files as PNG images via Typst and the Kitty graphics protocol.

## Requirements

### External tools

mdx shells out to two external programs. Both must be in `PATH`:

| Tool    | Purpose                          | Install (macOS)          | Install (Linux)                                  |
|---------|----------------------------------|--------------------------|--------------------------------------------------|
| pandoc  | Markdown → Typst source          | `brew install pandoc`    | `sudo apt install pandoc`                        |
| typst   | Typst source → PNG               | `brew install typst`     | `cargo install typst-cli` / `sudo pacman -S typst` |

Run `mdx` and it will check automatically; missing tools produce a clear error.

PDF export uses the pinned `@preview/elegant-paper:0.1.0` Typst template and
its `zh-kit` dependency. Typst downloads these packages on the first PDF
export and keeps them in its package cache. PDF export uses normal A4
pagination by default; use `--continuous` for one continuous long page.

### Terminal

mdx uses the **Kitty graphics protocol**. Supported terminals:

- Kitty
- Ghostty
- WezTerm
- rio
- Otty

Other terminals (iTerm2, Terminal.app, Alacritty, GNOME Terminal) will not display images.

### Platform

Unix-like systems only (macOS, Linux). Uses `termios`, `ioctl(TIOCGWINSZ)`, and `SIGWINCH`.

## Fonts

mdx looks for font files in `fonts/` (or the directory specified by `MDX_FONT_PATH`). If the directory is empty, it falls back to system-installed fonts and prints a warning.

### Expected font families

**Body text:** `Noto Sans CJK SC` → `Noto Sans` → `LXGW WenKai`

**Code / monospace:** `JetBrains Mono` → `Fira Code` → `Menlo`

### Bundling fonts

Download the OFL-licensed fonts and place `.ttf`/`.otf` files directly in `fonts/`:

- Noto Sans + Noto Sans CJK SC: <https://fonts.google.com/noto>
- JetBrains Mono: <https://www.jetbrains.com/lp/mono/>
- LXGW WenKai: <https://github.com/lxgw/LxgwWenKai>

## Usage

```bash
# Render a Markdown file → interactive pager
mdx file.md

# Export a paginated PDF next to the Markdown file → file.pdf
mdx --pdf file.md

# Export one continuous long page instead
mdx --pdf --continuous file.md

# Explicitly request normal A4 pagination
mdx --pdf --paginate file.md

# The file is watched; saving changes in your editor re-renders
# automatically, keeping your reading position (by progress %).
```

In batch mode mdx starts an interactive pager. In streaming mode (no file arguments)
it renders each Markdown block live as the child produces it.

### Pager keys

| Key      | Action        |
|----------|---------------|
| ↓ → j n  | Next page     |
| ↑ ← k p  | Previous page |
| g        | First page    |
| G        | Last page     |
| space / q | Quit         |

In watch mode (single `.md` file) the status bar shows `[watch]` and the
document re-renders on save, keeping your reading progress (by percentage).

### macOS App

Build the Finder-integrated macOS app:

```bash
bash macos-app/build_app.sh
open "mdx PDF.app"
```

The app registers itself as a handler for `.md`, `.markdown`, and `.mdown` files,
so it can be selected from Finder's "Open With" menu. It also accepts files
passed by a third-party Finder context-menu tool. The app automatically writes
the PDF next to the source file. The build script bundles the locally installed
`pandoc` and `typst` binaries when available, so exports launched outside a
shell do not depend on the shell's `PATH`. In the app, choose `分页` or `不分页`
before exporting; the choice is remembered for the next Finder invocation.

## Configuration

| Env var          | Default | Description                        |
|------------------|---------|------------------------------------|
| `MDX_FONT_PATH`  | `fonts` | Directory to scan for font files   |
| `MDX_TYPST_PPI`  | `200`   | PNG resolution (72–600)            |
| `MDX_TYPST_FONT_PT` | auto | Override font size in points       |
| `MDX_FORCE_KITTY` | auto   | `1`/`0` to force-enable/disable KGP |
| `MDX_FG`         | auto    | Foreground color as `rgb:rr/gg/bb` |
| `MDX_BG`         | auto    | Background color as `rgb:rr/gg/bb` |

## Build

```bash
cargo build --release
# Binary at target/release/mdx
```
