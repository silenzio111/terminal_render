#!/bin/bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
APP_NAME="mdx PDF.app"
APP_DIR="$ROOT_DIR/$APP_NAME"
BUILD_DIR="$ROOT_DIR/target/macos-app"
CLI_BINARY="$ROOT_DIR/target/release/mdx"
ICON_SOURCE="$ROOT_DIR/macos-app/AppIcon.svg"
ICON_SOURCE_PNG="$ROOT_DIR/macos-app/AppIcon.png"
ICON_FALLBACK="$ROOT_DIR/macos-app/AppIcon.icns"
ICONSET_DIR="$BUILD_DIR/AppIcon.iconset"

mkdir -p "$BUILD_DIR"

printf '构建 mdx 排版引擎...\n'
cargo build --release --manifest-path "$ROOT_DIR/Cargo.toml"

printf '构建 macOS App...\n'
swift build \
    --package-path "$ROOT_DIR/macos-app" \
    -c release
SWIFT_BIN_DIR="$(swift build \
    --package-path "$ROOT_DIR/macos-app" \
    -c release \
    --show-bin-path)"
APP_BINARY="$SWIFT_BIN_DIR/mdx-pdf-app"

printf '组装 App Bundle...\n'
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources/bin"

cp "$ROOT_DIR/macos-app/Info.plist" "$APP_DIR/Contents/Info.plist"
cp "$APP_BINARY" "$APP_DIR/Contents/MacOS/mdx-pdf-app"
cp "$CLI_BINARY" "$APP_DIR/Contents/Resources/mdx"
chmod +x "$APP_DIR/Contents/MacOS/mdx-pdf-app" "$APP_DIR/Contents/Resources/mdx"

copy_tool() {
    local tool_name="$1"
    local tool_path
    tool_path="$(command -v "$tool_name" || true)"
    if [ -n "$tool_path" ] && [ -x "$tool_path" ]; then
        cp "$tool_path" "$APP_DIR/Contents/Resources/bin/$tool_name"
        chmod +x "$APP_DIR/Contents/Resources/bin/$tool_name"
        printf '已捆绑 %s: %s\n' "$tool_name" "$tool_path"
    else
        printf '警告: 未找到 %s，App 将依赖运行时 PATH。\n' "$tool_name" >&2
    fi
}

copy_tool pandoc
copy_tool typst

if command -v iconutil >/dev/null 2>&1 && command -v magick >/dev/null 2>&1 && [ -f "$ICON_SOURCE_PNG" ]; then
    rm -rf "$ICONSET_DIR"
    mkdir -p "$ICONSET_DIR"

    render_icon() {
        local pixel_size="$1"
        local file_name="$2"
        magick "$ICON_SOURCE_PNG" \
            -filter Lanczos \
            -resize "${pixel_size}x${pixel_size}!" \
            -strip \
            "$ICONSET_DIR/$file_name"
    }

    render_icon 16 "icon_16x16.png"
    render_icon 32 "icon_16x16@2x.png"
    render_icon 32 "icon_32x32.png"
    render_icon 64 "icon_32x32@2x.png"
    render_icon 128 "icon_128x128.png"
    render_icon 256 "icon_128x128@2x.png"
    render_icon 256 "icon_256x256.png"
    render_icon 512 "icon_256x256@2x.png"
    render_icon 512 "icon_512x512.png"
    render_icon 1024 "icon_512x512@2x.png"

    iconutil --convert icns \
        --output "$APP_DIR/Contents/Resources/AppIcon.icns" \
        "$ICONSET_DIR"
    printf '已使用生成图像制作 App 图标。\n'
elif command -v rsvg-convert >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
    rm -rf "$ICONSET_DIR"
    mkdir -p "$ICONSET_DIR"

    render_icon() {
        local pixel_size="$1"
        local file_name="$2"
        rsvg-convert -w "$pixel_size" -h "$pixel_size" "$ICON_SOURCE" \
            -o "$ICONSET_DIR/$file_name"
    }

    render_icon 16 "icon_16x16.png"
    render_icon 32 "icon_16x16@2x.png"
    render_icon 32 "icon_32x32.png"
    render_icon 64 "icon_32x32@2x.png"
    render_icon 128 "icon_128x128.png"
    render_icon 256 "icon_128x128@2x.png"
    render_icon 256 "icon_256x256.png"
    render_icon 512 "icon_256x256@2x.png"
    render_icon 512 "icon_512x512.png"
    render_icon 1024 "icon_512x512@2x.png"

    iconutil --convert icns \
        --output "$APP_DIR/Contents/Resources/AppIcon.icns" \
        "$ICONSET_DIR"
    printf '已使用 SVG 制作 App 图标。\n'
elif [ -f "$ICON_FALLBACK" ]; then
    cp "$ICON_FALLBACK" "$APP_DIR/Contents/Resources/AppIcon.icns"
    printf '已复制预生成 App 图标。\n'
else
    printf '警告: 未找到 SVG 图标转换工具，App 将使用系统默认图标。\n' >&2
fi

if command -v codesign >/dev/null 2>&1; then
    codesign --force --deep --sign - "$APP_DIR" >/dev/null
fi

LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [ -x "$LSREGISTER" ]; then
    "$LSREGISTER" -f "$APP_DIR" >/dev/null
    printf '已注册 Markdown 文件处理程序。\n'
fi

printf '\n已生成: %s\n' "$APP_DIR"
printf '可在 Finder 的“打开方式”中使用，也可将此 App 的路径交给右键工具。\n'
