#!/usr/bin/env bash
# 把 GTK4 版 dmcl.exe 和运行时依赖打成一个自包含 zip（在 MSYS2 MINGW64 里运行）。
set -euo pipefail

root="$(pwd)"
out="$root/dist/dmcl-gtk4-windows"
zipfile="$root/dist/dmcl-gtk4-windows.zip"
binary="target/x86_64-pc-windows-gnu/release/dmcl.exe"

rm -rf "$out" "$zipfile"
mkdir -p "$out" "$root/dist"

cp "$binary" "$out/dmcl.exe"

# 递归收集所有 MinGW 运行库（系统 DLL 不复制）。
declare -A copied=()
collect_dlls() {
  local queue=("$1")
  while ((${#queue[@]} > 0)); do
    local file="${queue[0]}"
    queue=("${queue[@]:1}")
    while IFS= read -r dll; do
      [ -n "$dll" ] || continue
      local name
      name="$(basename "$dll")"
      if [ -z "${copied[$name]:-}" ]; then
        copied[$name]=1
        cp "$dll" "$out/$name"
        queue+=("$dll")
      fi
    done < <(ldd "$file" 2>/dev/null | awk '$2 == "=>" && $3 ~ /mingw64\/bin\// { print $3 }')
  done
}

collect_dlls "$binary"

if [ "${#copied[@]}" -eq 0 ]; then
  echo "没有收集到任何 MinGW DLL，ldd 可能不可用" >&2
  exit 1
fi

# GTK / GLib 运行时数据。
mkdir -p "$out/share" "$out/lib"
cp -a /mingw64/share/glib-2.0 "$out/share/"
if [ -d /mingw64/share/icons ]; then
  cp -a /mingw64/share/icons "$out/share/"
fi
for dir in gtk-4.0 libadwaita-1 themes; do
  if [ -d "/mingw64/share/$dir" ]; then
    cp -a "/mingw64/share/$dir" "$out/share/"
  fi
done
for dir in gdk-pixbuf-2.0 gio gtk-4.0; do
  if [ -d "/mingw64/lib/$dir" ]; then
    mkdir -p "$out/lib/$dir"
    cp -a "/mingw64/lib/$dir/." "$out/lib/$dir/"
  fi
done

# loaders / GIO modules / GTK 模块是动态加载的，单独收集它们依赖的 DLL。
while IFS= read -r dll; do
  collect_dlls "$dll"
done < <(find "$out/lib/gdk-pixbuf-2.0" "$out/lib/gio" "$out/lib/gtk-4.0" \
  -type f -name '*.dll' 2>/dev/null || true)

# 重新生成 schema / loader / GIO 模块缓存，避免写死构建机路径。
glib-compile-schemas "$out/share/glib-2.0/schemas"

if [ -d "$out/lib/gdk-pixbuf-2.0/2.10.0/loaders" ]; then
  cache="$out/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
  ( cd "$out" && GDK_PIXBUF_MODULEDIR="lib/gdk-pixbuf-2.0/2.10.0/loaders" gdk-pixbuf-query-loaders ) > "$cache"
  # 只保留 DLL 文件名；启动脚本会把 loaders 目录加进 PATH。
  sed -i -E 's|"[^"]*[\\/]([^"\\/]+)"|"\1"|g' "$cache"
fi

if [ -d "$out/lib/gio/modules" ]; then
  ( cd "$out/lib/gio/modules" && gio-querymodules . )
fi

# 启动器：设置 GTK 的运行时路径并默认启动 GUI。
{
  printf '@echo off\r\n'
  printf 'setlocal\r\n'
  printf 'cd /d "%%~dp0"\r\n'
  printf 'set "GTK_DATA_PREFIX=%%CD%%"\r\n'
  printf 'set "GTK_EXE_PREFIX=%%CD%%"\r\n'
  printf 'set "XDG_DATA_DIRS=%%CD%%\\share"\r\n'
  printf 'set "GSETTINGS_SCHEMA_DIR=%%CD%%\\share\\glib-2.0\\schemas"\r\n'
  printf 'set "GDK_PIXBUF_MODULE_FILE=%%CD%%\\lib\\gdk-pixbuf-2.0\\2.10.0\\loaders.cache"\r\n'
  printf 'set "GIO_MODULE_DIR=%%CD%%\\lib\\gio\\modules"\r\n'
  printf 'set "GTK_PATH=%%CD%%\\lib\\gtk-4.0"\r\n'
  printf 'set "PATH=%%CD%%;%%CD%%\\lib\\gdk-pixbuf-2.0\\2.10.0\\loaders;%%PATH%%"\r\n'
  printf 'start "" "%%~dp0dmcl.exe" gui %%*\r\n'
} > "$out/dmcl.cmd"

cat > "$out/README.txt" <<'EOF'
DMCL (GTK4)
===========

双击 dmcl.cmd 启动图形界面（等价于 dmcl.exe gui）。

本目录已经包含 GTK4 / libadwaita 的运行时 DLL 和数据，不需要另外安装 MSYS2。
需要 64 位 Windows 10/11。
EOF

(
  cd "$root/dist"
  zip -qr "dmcl-gtk4-windows.zip" "dmcl-gtk4-windows"
)

echo "已生成 $zipfile"
