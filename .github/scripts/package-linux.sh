#!/usr/bin/env bash
# 把 GTK4 版 dmcl 打成自包含的 AppImage。
#
# 依赖（Ubuntu）：libgtk-4-dev libadwaita-1-dev libgtk-4-bin libglib2.0-bin
#                 libgdk-pixbuf-2.0-bin adwaita-icon-theme gsettings-desktop-schemas
#                 pkg-config patchelf squashfs-tools desktop-file-utils
set -euo pipefail

root="$(pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
appdir="$work/AppDir"
tools="$work/tools"
output="$root/dist/dmcl-gtk4-linux.AppImage"

rm -rf "$output"
mkdir -p "$appdir/usr/bin" "$tools" "$root/dist"

cp target/release/dmcl "$appdir/usr/bin/dmcl"

cat > "$appdir/dmcl.desktop" <<'EOF'
[Desktop Entry]
Name=DMCL
Comment=Minecraft launcher
Exec=dmcl
Icon=dmcl
Type=Application
Categories=Game;
Terminal=false
EOF

# 临时图标，之后可以换成正式美术资源。
cat > "$appdir/dmcl.svg" <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">
  <rect width="256" height="256" rx="56" fill="#3584e4"/>
  <text x="128" y="168" font-family="sans-serif" font-size="120" font-weight="bold"
        fill="#ffffff" text-anchor="middle">D</text>
</svg>
EOF

# 符号图标来自 Adwaita 图标主题，一起打进 AppDir。
if [ -d /usr/share/icons/Adwaita ]; then
  mkdir -p "$appdir/usr/share/icons"
  cp -R /usr/share/icons/Adwaita "$appdir/usr/share/icons/"
fi

curl -fsSL -o "$tools/linuxdeploy" \
  https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
curl -fsSL -o "$tools/linuxdeploy-plugin-gtk" \
  https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh
curl -fsSL -o "$tools/appimagetool" \
  https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x "$tools/linuxdeploy" "$tools/linuxdeploy-plugin-gtk" "$tools/appimagetool"

export APPIMAGE_EXTRACT_AND_RUN=1
export DEPLOY_GTK_VERSION=4
export PATH="$tools:$PATH"
export ARCH=x86_64

"$tools/linuxdeploy" --appdir "$appdir" \
  --plugin gtk \
  --desktop-file "$appdir/dmcl.desktop" \
  --icon-file "$appdir/dmcl.svg"

"$tools/appimagetool" "$appdir" "$output"

echo "已生成 $output"
