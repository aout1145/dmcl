//! Fabric / Forge / NeoForge 三种加载器共用的收尾工具。

use dmcl_mcmeta::source::Mirror;
use dmcl_mcmeta::vanilla::VersionJson;

/// 收尾：写盘前把 URL 模板解析回官方地址，并记下客户端 jar 名。
pub(crate) fn finish_version_json(json: &mut VersionJson, version_name: &str, own_jar: bool) {
    json.resolve_urls(Mirror::Official);
    json.inherits_from = None;
    json.id = Some(version_name.to_string());
    if own_jar {
        json.jar = Some(version_name.to_string());
    }
}
