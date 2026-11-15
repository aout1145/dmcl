#!/usr/bin/env bash
# 把 GTK4 版 dmcl.exe 和运行时依赖准备成一个自包含目录（CI 再上传为 artifact）。
set -euo pipefail

root="$(pwd)"
out="$root/dist/dmcl-gtk4-windows"
binary="target/release/dmcl.exe"

rm -rf "$out"
mkdir -p "$out" "$root/dist"

cp "$binary" "$out/dmcl.exe"

# 递归收集所有 CLANG64 运行库（UCRT 属于系统 DLL，不复制）。
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
    done < <(ldd "$file" 2>/dev/null | awk '$2 == "=>" && $3 ~ /clang64\/bin\// { print $3 }')
  done
}

collect_dlls "$binary"

if [ "${#copied[@]}" -eq 0 ]; then
  echo "没有收集到任何 CLANG64 DLL，ldd 可能不可用" >&2
  exit 1
fi

# GTK / GLib 运行时数据。
mkdir -p "$out/share" "$out/lib"
cp -a /clang64/share/glib-2.0 "$out/share/"
if [ -d /clang64/share/icons ]; then
  cp -a /clang64/share/icons "$out/share/"
fi
for dir in gtk-4.0 libadwaita-1 themes; do
  if [ -d "/clang64/share/$dir" ]; then
    cp -a "/clang64/share/$dir" "$out/share/"
  fi
done
for dir in gdk-pixbuf-2.0 gio gtk-4.0; do
  if [ -d "/clang64/lib/$dir" ]; then
    mkdir -p "$out/lib/$dir"
    cp -a "/clang64/lib/$dir/." "$out/lib/$dir/"
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
  # 用相对目录生成，缓存里就是相对 exe 目录的路径；MSYS2 的 gdk-pixbuf 可重定位，
  # 运行时会以 DLL 所在目录为前缀解析。
  ( cd "$out" && GDK_PIXBUF_MODULEDIR="lib/gdk-pixbuf-2.0/2.10.0/loaders" gdk-pixbuf-query-loaders ) > "$cache"
fi

if [ -d "$out/lib/gio/modules" ]; then
  ( cd "$out/lib/gio/modules" && gio-querymodules . )
fi

cat > "$out/README.txt" <<'EOF'
DMCL (GTK4)
===========

双击 dmcl.exe 启动图形界面（等价于 dmcl.exe gui）。

本目录已经包含 GTK4 / libadwaita 的运行时 DLL 和数据，不需要另外安装 MSYS2。
GLib / GIO / gdk-pixbuf 会按 DLL 所在目录查找 share 与 lib 下的运行时数据，
所以直接运行 dmcl.exe 即可，不需要额外的启动脚本。
需要 64 位 Windows 10/11。
EOF

echo "已准备 $out"
