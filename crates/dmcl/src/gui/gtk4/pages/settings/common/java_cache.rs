//! Java 探测结果的进程内缓存。
//!
//! Java 管理页和游戏设置页共享同一份 [`JavaProbeCache`]：切换分类时不会为同一个
//! 可执行文件反复执行 `java -version`。缓存只在主线程访问，不写回配置。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use dmcl_launch::JavaDetails;
use dmcl_mcmeta::layout::absolute;

/// 按绝对路径缓存的 Java 探测结果。
#[derive(Default)]
pub(crate) struct JavaProbeCache {
    entries: RefCell<HashMap<PathBuf, JavaDetails>>,
}

impl JavaProbeCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 已缓存的探测结果；路径按绝对路径比较。
    pub(crate) fn get(&self, path: &Path) -> Option<JavaDetails> {
        self.entries.borrow().get(&absolute(path)).cloned()
    }

    /// 记下一次探测结果。
    pub(crate) fn insert(&self, path: PathBuf, details: JavaDetails) {
        self.entries.borrow_mut().insert(absolute(&path), details);
    }

    /// 丢掉配置里已经不存在的 Java，避免缓存无限增长。
    pub(crate) fn retain(&self, paths: &[PathBuf]) {
        let keep: HashSet<PathBuf> = paths.iter().map(absolute).collect();
        self.entries
            .borrow_mut()
            .retain(|path, _| keep.contains(path));
    }
}
