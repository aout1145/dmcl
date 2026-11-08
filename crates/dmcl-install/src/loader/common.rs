//! Fabric / Forge / NeoForge 三种加载器共用的收尾工具。

use dmcl_mcmeta::vanilla::VersionJson;

/// 收尾：写盘前把 URL 模板解析回官方地址，并让主 jar 指向自己的版本目录。
pub(crate) fn finish_version_json(json: &mut VersionJson, version_name: &str) {
    json.resolve_official_urls();
    json.inherits_from = None;
    json.id = Some(version_name.to_string());
    json.jar = Some(version_name.to_string());
}
