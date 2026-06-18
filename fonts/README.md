# Bundled Fonts

`mdx` loads fonts from this directory via Typst's `--font-path`. Place `.ttf` or `.otf` font files here so the renderer does not depend on system-installed fonts.

## Expected font families

The renderer looks for the following font family names (in order):

### Body text
1. `Noto Sans CJK SC` — Chinese/Japanese/Korean + Latin sans-serif
2. `Noto Sans` — Latin sans-serif fallback
3. `LXGW WenKai` — Chinese serif/handwriting fallback

### Code / monospace
1. `JetBrains Mono`
2. `Fira Code`
3. `Menlo`

## Suggested font downloads (OFL licensed)

- **Noto Sans + Noto Sans CJK SC**: https://fonts.google.com/noto
- **JetBrains Mono**: https://www.jetbrains.com/lp/mono/
- **LXGW WenKai**: https://github.com/lxgw/LxgwWenKai

After downloading, extract the font files (`.ttf` or `.otf`) directly into this directory. You do not need to maintain subdirectories; Typst scans the whole `--font-path` recursively.

## Custom font path

You can also point `mdx` at a different directory with the environment variable:

```bash
MDX_FONT_PATH=/path/to/your/fonts mdx cat file.md
```

## Licensing reminder

Only redistribute fonts whose licenses allow it. The families listed above are licensed under the SIL Open Font License (OFL) and are safe to bundle with your own distributions of `mdx`.
