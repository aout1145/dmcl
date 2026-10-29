//! 编译期生成的构建信息。
//!
//! 由 `build.rs` 里的 `built` 写入 `OUT_DIR/built.rs`；这里只做一次 include，
//! 具体字段（`BUILT_TIME_UTC` / `PKG_VERSION` 等）见生成的文件。

include!(concat!(env!("OUT_DIR"), "/built.rs"));
