//! maven 坐标处理
//! （`libNameToPath` / `libNameGetName` / `libNameGetVersion`）。
//!
//! 刻意手写：crates.io 上没有维护良好、又能处理
//! `group:artifact:version[:classifier][@ext]` 这种自由形式的库，
//! 而这三段逻辑各只有十几行纯字符串处理。

/// `libNameToPath` —— maven 坐标转成仓库里的相对路径。
///
/// ```text
/// com.example:demo:1.0                  -> com/example/demo/1.0/demo-1.0.jar
/// org.lwjgl:lwjgl:3.3.1:natives-linux@zip
///                                       -> org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.zip
/// ```
pub fn lib_name_to_path(libname: &str) -> String {
    let libname = if !libname.contains('@') {
        format!("{libname}@jar")
    } else {
        libname.to_string()
    };
    let mut rawstr = libname.splitn(2, '@');
    let coord = rawstr.next().unwrap_or("");
    let ext = rawstr.next().unwrap_or("jar");
    let rawname: Vec<&str> = coord.split(':').collect();
    let pkg = rawname.first().copied().unwrap_or("");
    let name = rawname.get(1).copied().unwrap_or("");
    let mut path = format!(
        "{}/{}/{}/{}",
        pkg.replace('.', "/"),
        name,
        rawname.get(2).copied().unwrap_or(""),
        name
    );
    for part in rawname.iter().skip(2) {
        path.push('-');
        path.push_str(part);
    }
    path.push('.');
    path.push_str(ext);
    path
}

/// `libNameGetName` —— `group:artifact[:classifier...]`（去掉版本）。
pub fn lib_name_get_name(libname: &str) -> String {
    let rawname: Vec<&str> = libname.split(':').collect();
    let mut name = format!(
        "{}:{}",
        rawname.first().copied().unwrap_or(""),
        rawname.get(1).copied().unwrap_or("")
    );
    if rawname.len() > 3 {
        for part in rawname.iter().skip(3) {
            name.push(':');
            name.push_str(part);
        }
    }
    name
}

/// `libNameGetVersion` —— maven 坐标里的版本部分。
pub fn lib_name_get_version(libname: &str) -> String {
    libname
        .split('@')
        .next()
        .unwrap_or("")
        .split(':')
        .nth(2)
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_coordinates_map_to_repository_paths() {
        assert_eq!(
            lib_name_to_path("com.example:demo:1.0"),
            "com/example/demo/1.0/demo-1.0.jar"
        );
        assert_eq!(
            lib_name_to_path("net.minecraftforge:forge:1.20.1-47.2.0"),
            "net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0.jar"
        );
        assert_eq!(
            lib_name_to_path("org.lwjgl:lwjgl:3.3.1:natives-linux@zip"),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.zip"
        );
    }

    #[test]
    fn maven_name_and_version_keep_classifiers() {
        assert_eq!(
            lib_name_get_name("org.lwjgl:lwjgl:3.3.1:natives-linux"),
            "org.lwjgl:lwjgl:natives-linux"
        );
        assert_eq!(
            lib_name_get_name("com.example:demo:1.0"),
            "com.example:demo"
        );
        assert_eq!(lib_name_get_version("com.example:demo:1.0@jar"), "1.0");
    }
}
