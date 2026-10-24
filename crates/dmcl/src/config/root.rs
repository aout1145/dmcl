//! 具名的 Minecraft 根目录。

use std::path::PathBuf;

use anyhow::{anyhow, Result};

use super::Config;

/// 一个具名的 Minecraft 根目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinecraftRoot {
    /// `minecraft_roots` 里的名字。
    pub name: String,
    /// 已经绝对化的根目录路径。
    pub path: PathBuf,
}

impl Config {
    /// 所有配置里的具名根目录。
    pub fn roots(&self) -> impl Iterator<Item = MinecraftRoot> + '_ {
        self.minecraft_roots
            .iter()
            .map(|(name, path)| MinecraftRoot {
                name: name.clone(),
                path: path.clone(),
            })
    }

    /// 按名字取根目录。
    pub fn root(&self, name: &str) -> Result<MinecraftRoot> {
        self.minecraft_roots
            .get(name)
            .map(|path| MinecraftRoot {
                name: name.to_string(),
                path: path.clone(),
            })
            .ok_or_else(|| anyhow!("根目录不存在：{name}"))
    }

    /// `minecraft_root` 指向的默认根目录。
    pub fn default_root(&self) -> Result<MinecraftRoot> {
        self.minecraft_roots
            .get(&self.minecraft_root)
            .map(|path| MinecraftRoot {
                name: self.minecraft_root.clone(),
                path: path.clone(),
            })
            .ok_or_else(|| {
                anyhow!(
                    "配置项「minecraft_root」指向的根目录不存在：{}",
                    self.minecraft_root
                )
            })
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
