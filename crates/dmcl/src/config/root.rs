//! 具名的 Minecraft 根目录。
//!
//! 配置里保存的是原始路径（可含 [`CWD_PLACEHOLDER`] 或相对路径）；取用根目录时展开
//! 占位符并绝对化，因此保存配置不会把当前目录写死。

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use dmcl_mcmeta::layout::absolute;

use super::{Config, CWD_PLACEHOLDER};

impl Config {
    /// 所有具名根目录：名字 → 已展开的路径，保持配置里的顺序。
    pub fn roots(&self) -> Vec<(String, PathBuf)> {
        self.minecraft_roots
            .iter()
            .map(|(name, path)| (name.clone(), resolve(path)))
            .collect()
    }

    /// 按名字取已展开的根目录路径。
    pub fn root_path(&self, name: &str) -> Result<PathBuf> {
        self.minecraft_roots
            .get(name)
            .map(|path| resolve(path))
            .ok_or_else(|| anyhow!("根目录不存在：{name}"))
    }

    /// `minecraft_root` 指向的默认根目录名字。
    pub fn default_root_name(&self) -> Result<&str> {
        let name = self.minecraft_root.as_str();
        self.minecraft_roots
            .contains_key(name)
            .then_some(name)
            .ok_or_else(|| {
                anyhow!(
                    "配置项「minecraft_root」指向的根目录不存在：{}",
                    self.minecraft_root
                )
            })
    }

    /// `minecraft_root` 指向的默认根目录路径。
    pub fn default_root_path(&self) -> Result<PathBuf> {
        self.root_path(self.default_root_name()?)
    }

    /// 修改默认根目录；名字必须已经在 `minecraft_roots` 里。
    pub fn select_minecraft_root(&mut self, name: &str) -> Result<()> {
        if !self.minecraft_roots.contains_key(name) {
            return Err(anyhow!("根目录不存在：{name}"));
        }
        self.minecraft_root = name.to_string();
        Ok(())
    }
}

/// 展开 `${CWD}` 并绝对化。
fn resolve(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if !text.contains(CWD_PLACEHOLDER) {
        return absolute(path);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    absolute(text.replace(CWD_PLACEHOLDER, &cwd.to_string_lossy()))
}
