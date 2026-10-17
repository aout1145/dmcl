#!/usr/bin/env bash
# 把 GTK4 版 dmcl 打成自包含的 DMCL.app（在 macOS + Homebrew 环境里运行）。
#
# 依赖：brew install pkgconf gtk4 libadwaita adwaita-icon-theme gsettings-desktop-schemas dylibbundler
set -euo pipefail

root="$(pwd)"
app="$root/dist/DMCL.app"
res="$app/Contents/Resources"
frameworks="$app/Contents/Frameworks"
zipfile="$root/dist/dmcl-gtk4-macos.zip"
version="$(awk -F'"' '/^version = / { print $2; exit }' Cargo.toml)"

rm -rf "$app" "$zipfile"
mkdir -p "$app/Contents/MacOS" "$frameworks" "$res/share" "$res/lib" "$root/dist"

cp target/release/dmcl "$app/Contents/MacOS/dmcl-bin"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>DMCL</string>
  <key>CFBundleDisplayName</key><string>DMCL</string>
  <key>CFBundleIdentifier</key><string>dev.dmcl.launcher</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleExecutable</key><string>dmcl</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
</dict>
</plist>
EOF

gtk_prefix="$(brew --prefix gtk4)"
adw_prefix="$(brew --prefix libadwaita)"
glib_prefix="$(brew --prefix glib)"
pixbuf_prefix="$(brew --prefix gdk-pixbuf)"
icons_prefix="$(brew --prefix adwaita-icon-theme)"
gsettings_prefix="$(brew --prefix gsettings-desktop-schemas)"

# 合并 GLib schemas（GTK / libadwaita / glib / gsettings-desktop-schemas 各有一部分）。
mkdir -p "$res/share/glib-2.0/schemas"
for prefix in "$gtk_prefix" "$adw_prefix" "$glib_prefix" "$gsettings_prefix"; do
  if [ -d "$prefix/share/glib-2.0/schemas" ]; then
    cp -R "$prefix/share/glib-2.0/schemas/." "$res/share/glib-2.0/schemas/"
  fi
done
"$glib_prefix/bin/glib-compile-schemas" "$res/share/glib-2.0/schemas"

# 图标主题 / GTK 数据。
mkdir -p "$res/share/icons"
if [ -d "$icons_prefix/share/icons" ]; then
  cp -R "$icons_prefix/share/icons/." "$res/share/icons/"
fi
for prefix in "$gtk_prefix" "$adw_prefix"; do
  for dir in gtk-4.0 libadwaita-1 icons; do
    if [ -d "$prefix/share/$dir" ]; then
      mkdir -p "$res/share/$dir"
      cp -R "$prefix/share/$dir/." "$res/share/$dir/"
    fi
  done
done

# gdk-pixbuf loaders / GIO modules / GTK 模块（Homebrew 不一定都带，缺了就跳过）。
if [ -d "$pixbuf_prefix/lib/gdk-pixbuf-2.0" ]; then
  cp -R "$pixbuf_prefix/lib/gdk-pixbuf-2.0" "$res/lib/"
fi
mkdir -p "$res/lib/gio"
if [ -d "$glib_prefix/lib/gio/modules" ]; then
  cp -R "$glib_prefix/lib/gio/modules" "$res/lib/gio/"
fi
if [ -d "$gtk_prefix/lib/gtk-4.0" ]; then
  cp -R "$gtk_prefix/lib/gtk-4.0" "$res/lib/"
fi

# 主程序和模块依赖的 Homebrew dylib 全部收进 Contents/Frameworks。
dylibbundler -od -b \
  -x "$app/Contents/MacOS/dmcl-bin" \
  -d "$frameworks" \
  -p "@executable_path/../Frameworks/"

pixbuf_loaders="$res/lib/gdk-pixbuf-2.0/2.10.0/loaders"
if [ -d "$pixbuf_loaders" ]; then
  for so in "$pixbuf_loaders"/*.so; do
    [ -e "$so" ] || continue
    dylibbundler -od -b -x "$so" -d "$frameworks" -p "@loader_path/../../../../../Frameworks/"
  done
  (
    cd "$res"
    GDK_PIXBUF_MODULEDIR="lib/gdk-pixbuf-2.0/2.10.0/loaders" \
      "$pixbuf_prefix/bin/gdk-pixbuf-query-loaders" \
      > "$res/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
  )
  sed -i '' -E 's|"[^"]*/(lib/gdk-pixbuf-2.0/[^"]*)"|"\1"|g' \
    "$res/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
fi

if [ -d "$res/lib/gio/modules" ]; then
  for so in "$res/lib/gio/modules"/*.so; do
    [ -e "$so" ] || continue
    dylibbundler -od -b -x "$so" -d "$frameworks" -p "@loader_path/../../../../Frameworks/"
  done
  (
    cd "$res/lib/gio/modules"
    "$glib_prefix/bin/gio-querymodules" .
  )
fi

# 启动脚本：设置 GTK 的运行时路径并默认启动 GUI。
cat > "$app/Contents/MacOS/dmcl" <<'EOF'
#!/bin/bash
set -e
dir="$(cd "$(dirname "$0")" && pwd)"
res="$dir/../Resources"
export GTK_DATA_PREFIX="$res"
export GTK_EXE_PREFIX="$res"
export XDG_DATA_DIRS="$res/share:/usr/local/share:/usr/share"
export GSETTINGS_SCHEMA_DIR="$res/share/glib-2.0/schemas"
export GDK_PIXBUF_MODULE_FILE="$res/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
export GIO_MODULE_DIR="$res/lib/gio/modules"
export GTK_PATH="$res/lib/gtk-4.0"
export DYLD_FALLBACK_LIBRARY_PATH="$dir/../Frameworks:${DYLD_FALLBACK_LIBRARY_PATH:-}"
cd "$res"
exec "$dir/dmcl-bin" gui "$@"
EOF
chmod +x "$app/Contents/MacOS/dmcl"

# Ad-hoc 签名；没有开发者证书时至少让 Gatekeeper 少拦一层。
codesign --force --deep --sign - "$app"

ditto -c -k --keepParent "$app" "$zipfile"
echo "已生成 $zipfile"
