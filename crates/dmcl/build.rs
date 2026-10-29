//! 用 `built` 生成编译期信息（构建时间等），供 `dmcl version` 使用。

fn main() {
    built::write_built_file().expect("无法写入构建信息");
}
