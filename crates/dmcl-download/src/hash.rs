//! 摘要与校验：md5 / sha1 / sha256 / sha512。
//!
//! Minecraft 自己的文档通常只提供 sha1（Fabric 给的是 sha512），但下载层**不假设算法**：
//! [`Checksum`] 表示"某个算法 + 某个十六进制摘要"，校验接口对所有算法一视同仁。
//! 因此需要 md5 / sha256 的场景（自建源、第三方清单、离线校验）不必再改调用方。
//!
//! ```no_run
//! use dmcl_download::{Checksum, HashAlgorithm};
//!
//! let checksum = Checksum::sha256("2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae");
//! assert!(checksum.verify_bytes(b"foo"));
//!
//! // 也支持带算法前缀的字符串；不带前缀时按摘要长度推断
//! let parsed = Checksum::parse("md5:acbd18db4cc2f85cedef654fccc4a4d8").unwrap();
//! assert_eq!(parsed.algorithm(), HashAlgorithm::Md5);
//! ```

use std::fmt;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
// 三个 RustCrypto 实现共用同一个 `digest` trait，导入一次即可
use sha1::Digest;

/// 支持的摘要算法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HashAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl HashAlgorithm {
    /// 全部算法。
    pub const ALL: [HashAlgorithm; 4] = [
        HashAlgorithm::Md5,
        HashAlgorithm::Sha1,
        HashAlgorithm::Sha256,
        HashAlgorithm::Sha512,
    ];

    /// 规范名字（也是 [`Checksum`] 显示时的前缀）。
    pub fn as_str(self) -> &'static str {
        match self {
            HashAlgorithm::Md5 => "md5",
            HashAlgorithm::Sha1 => "sha1",
            HashAlgorithm::Sha256 => "sha256",
            HashAlgorithm::Sha512 => "sha512",
        }
    }

    /// 摘要的字节数。
    pub fn digest_len(self) -> usize {
        match self {
            HashAlgorithm::Md5 => 16,
            HashAlgorithm::Sha1 => 20,
            HashAlgorithm::Sha256 => 32,
            HashAlgorithm::Sha512 => 64,
        }
    }

    /// 摘要的十六进制字符数。
    pub fn hex_len(self) -> usize {
        self.digest_len() * 2
    }

    /// 按名字识别算法（大小写不敏感，接受 `sha-1` 这类写法）。
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().replace('-', "").as_str() {
            "md5" => Some(HashAlgorithm::Md5),
            "sha1" => Some(HashAlgorithm::Sha1),
            "sha256" => Some(HashAlgorithm::Sha256),
            "sha512" => Some(HashAlgorithm::Sha512),
            _ => None,
        }
    }

    /// 由摘要长度反推算法。
    pub fn from_hex_len(hex_len: usize) -> Option<Self> {
        HashAlgorithm::ALL
            .into_iter()
            .find(|algorithm| algorithm.hex_len() == hex_len)
    }
}

impl fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一个校验值：算法 + 十六进制摘要（内部一律小写）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksum {
    algorithm: HashAlgorithm,
    hex: String,
}

impl Checksum {
    /// 直接构造（去掉首尾空白并转成小写）。
    pub fn new(algorithm: HashAlgorithm, hex: impl Into<String>) -> Self {
        Self {
            algorithm,
            hex: hex.into().trim().to_ascii_lowercase(),
        }
    }

    pub fn md5(hex: impl Into<String>) -> Self {
        Self::new(HashAlgorithm::Md5, hex)
    }

    pub fn sha1(hex: impl Into<String>) -> Self {
        Self::new(HashAlgorithm::Sha1, hex)
    }

    pub fn sha256(hex: impl Into<String>) -> Self {
        Self::new(HashAlgorithm::Sha256, hex)
    }

    pub fn sha512(hex: impl Into<String>) -> Self {
        Self::new(HashAlgorithm::Sha512, hex)
    }

    /// 解析一个校验值。
    ///
    /// * 带算法前缀：`sha256:abcd…`、`sha-1=abcd…`；
    /// * 不带前缀：按十六进制长度推断（32 → md5，40 → sha1，64 → sha256，128 → sha512）。
    ///
    /// 摘要必须是十六进制且长度与算法相符，否则报错 —— 写错的校验值应该立刻被发现，
    /// 而不是静默地"永远校验失败"。
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.is_empty() {
            return Err(anyhow!("校验值为空"));
        }

        // `算法:摘要` 或 `算法=摘要`
        let (algorithm, hex) = match text.split_once([':', '=']) {
            Some((name, hex)) => {
                let algorithm = HashAlgorithm::from_name(name)
                    .ok_or_else(|| anyhow!("不认识的摘要算法: {name}"))?;
                (algorithm, hex.trim())
            }
            None => {
                let algorithm = HashAlgorithm::from_hex_len(text.len()).ok_or_else(|| {
                    anyhow!(
                        "无法从长度 {} 推断摘要算法（可用 md5/sha1/sha256/sha512）",
                        text.len()
                    )
                })?;
                (algorithm, text)
            }
        };

        if hex.len() != algorithm.hex_len() {
            return Err(anyhow!(
                "{algorithm} 的摘要应当是 {} 位十六进制，实际是 {} 位",
                algorithm.hex_len(),
                hex.len()
            ));
        }
        if !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(anyhow!("{algorithm} 的摘要里含有非十六进制字符"));
        }

        Ok(Self::new(algorithm, hex))
    }

    /// 这个校验值用的算法。
    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    /// 小写的十六进制摘要。
    pub fn hex(&self) -> &str {
        &self.hex
    }

    /// 算一遍这个校验值对应的摘要（用于对比 / 调试）。
    pub fn compute(&self, data: &[u8]) -> String {
        hash_bytes(self.algorithm, data)
    }

    /// 字节是否匹配。
    pub fn verify_bytes(&self, data: &[u8]) -> bool {
        self.compute(data) == self.hex
    }

    /// 文件是否匹配。
    pub async fn verify_file(&self, path: impl AsRef<Path>) -> Result<bool> {
        Ok(hash_file(path, self.algorithm).await? == self.hex)
    }
}

impl fmt::Display for Checksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algorithm, self.hex)
    }
}

impl std::str::FromStr for Checksum {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

// ---------------------------------------------------------------------------
// 摘要计算
// ---------------------------------------------------------------------------

/// 一个字节串的十六进制摘要。
pub fn hash_bytes(algorithm: HashAlgorithm, data: &[u8]) -> String {
    let mut hasher = AnyHasher::new(algorithm);
    hasher.update(data);
    hasher.finish()
}

/// 一个文件的十六进制摘要（流式读取，不把整个文件读进内存）。
pub async fn hash_file(path: impl AsRef<Path>, algorithm: HashAlgorithm) -> Result<String> {
    let path = path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<String> {
        use std::io::Read;

        let mut file =
            std::fs::File::open(&path).with_context(|| format!("无法打开 {}", path.display()))?;
        let mut hasher = AnyHasher::new(algorithm);
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hasher.finish())
    })
    .await
    .context("计算文件摘要的任务失败")?
}

// ---------------------------------------------------------------------------
// 本地文件判断
// ---------------------------------------------------------------------------

/// 文件是否存在、且大小符合预期。
async fn size_matches(path: &Path, size: Option<u64>) -> bool {
    match size {
        None => true,
        Some(expected) => match tokio::fs::metadata(path).await {
            Ok(metadata) => metadata.len() == expected,
            Err(_) => false,
        },
    }
}

/// 本地这个文件是否已经「存在且完整」。
///
/// * 给了 `checksum` —— 摘要必须一致（算法由 [`Checksum`] 决定）；
/// * 只给了 `size` —— 大小一致即可；
/// * 两个都没给 —— 文件存在就算数。
pub async fn file_ok(
    path: impl AsRef<Path>,
    checksum: Option<&Checksum>,
    size: Option<u64>,
) -> bool {
    let path = path.as_ref();
    if tokio::fs::metadata(path).await.is_err() {
        return false;
    }
    match checksum {
        Some(checksum) => checksum.verify_file(path).await.unwrap_or(false),
        None => size_matches(path, size).await,
    }
}

// ---------------------------------------------------------------------------
// 内部：把四种算法收进一个枚举
// ---------------------------------------------------------------------------

enum AnyHasher {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
}

impl AnyHasher {
    fn new(algorithm: HashAlgorithm) -> Self {
        match algorithm {
            HashAlgorithm::Md5 => AnyHasher::Md5(md5::Md5::new()),
            HashAlgorithm::Sha1 => AnyHasher::Sha1(sha1::Sha1::new()),
            HashAlgorithm::Sha256 => AnyHasher::Sha256(sha2::Sha256::new()),
            HashAlgorithm::Sha512 => AnyHasher::Sha512(sha2::Sha512::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            AnyHasher::Md5(hasher) => hasher.update(data),
            AnyHasher::Sha1(hasher) => hasher.update(data),
            AnyHasher::Sha256(hasher) => hasher.update(data),
            AnyHasher::Sha512(hasher) => hasher.update(data),
        }
    }

    fn finish(self) -> String {
        match self {
            AnyHasher::Md5(hasher) => hex::encode(hasher.finalize()),
            AnyHasher::Sha1(hasher) => hex::encode(hasher.finalize()),
            AnyHasher::Sha256(hasher) => hex::encode(hasher.finalize()),
            AnyHasher::Sha512(hasher) => hex::encode(hasher.finalize()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha512_matches_its_known_digest() {
        assert_eq!(
            hash_bytes(HashAlgorithm::Sha512, b"abc"),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
    }

    #[test]
    fn every_algorithm_matches_its_known_digests() {
        // 空输入与 "abc" 的公开测试向量
        assert_eq!(
            hash_bytes(HashAlgorithm::Md5, b""),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
        assert_eq!(
            hash_bytes(HashAlgorithm::Md5, b"abc"),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            hash_bytes(HashAlgorithm::Sha1, b""),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            hash_bytes(HashAlgorithm::Sha1, b"abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hash_bytes(HashAlgorithm::Sha256, b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash_bytes(HashAlgorithm::Sha256, b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn checksums_parse_with_and_without_an_algorithm_prefix() {
        let parsed = Checksum::parse(
            "sha256:2C26B46B68FFC68FF99B453C1D30413413422D706483BFA0F98A5E886266E7AE",
        )
        .unwrap();
        assert_eq!(parsed.algorithm(), HashAlgorithm::Sha256);
        assert_eq!(
            parsed.hex(),
            "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae"
        );
        assert!(parsed.verify_bytes(b"foo"));

        // 不带前缀时按长度推断
        assert_eq!(
            Checksum::parse("acbd18db4cc2f85cedef654fccc4a4d8")
                .unwrap()
                .algorithm(),
            HashAlgorithm::Md5
        );
        assert_eq!(
            Checksum::parse("a9993e364706816aba3e25717850c26c9cd0d89d")
                .unwrap()
                .algorithm(),
            HashAlgorithm::Sha1
        );
        // `sha-1=…` 这种写法也接受
        assert_eq!(
            Checksum::parse("sha-1=a9993e364706816aba3e25717850c26c9cd0d89d")
                .unwrap()
                .algorithm(),
            HashAlgorithm::Sha1
        );
    }

    #[test]
    fn bad_checksums_are_rejected_early() {
        assert!(Checksum::parse("").is_err());
        assert!(Checksum::parse("nope:abcd").is_err());
        assert!(Checksum::parse("sha1:1234").is_err()); // 长度不对
        assert!(Checksum::parse("12345").is_err()); // 无法推断
        assert!(Checksum::parse("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_err()); // 非十六进制
    }

    #[test]
    fn display_round_trips_through_parse() {
        let checksum = Checksum::sha256("AB".repeat(32));
        assert_eq!(checksum.to_string(), format!("sha256:{}", "ab".repeat(32)));
        assert_eq!(Checksum::parse(&checksum.to_string()).unwrap(), checksum);
        assert_eq!(
            "md5:acbd18db4cc2f85cedef654fccc4a4d8"
                .parse::<Checksum>()
                .unwrap()
                .algorithm(),
            HashAlgorithm::Md5
        );
    }

    #[test]
    fn algorithms_can_be_recognised_by_name_or_length() {
        assert_eq!(
            HashAlgorithm::from_name("SHA256"),
            Some(HashAlgorithm::Sha256)
        );
        assert_eq!(HashAlgorithm::from_name("md-5"), Some(HashAlgorithm::Md5));
        assert_eq!(HashAlgorithm::from_name("crc32"), None);
        assert_eq!(HashAlgorithm::from_hex_len(40), Some(HashAlgorithm::Sha1));
        assert_eq!(HashAlgorithm::from_hex_len(41), None);
        assert_eq!(HashAlgorithm::Sha1.hex_len(), 40);
        assert_eq!(HashAlgorithm::Md5.to_string(), "md5");
    }

    #[tokio::test]
    async fn files_are_checked_by_any_algorithm_or_by_size() {
        let dir = std::env::temp_dir().join(format!("dmcl-hash-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("a.txt");
        tokio::fs::write(&path, b"abc").await.unwrap();

        for algorithm in HashAlgorithm::ALL {
            let checksum = Checksum::new(algorithm, hash_bytes(algorithm, b"abc"));
            assert!(file_ok(&path, Some(&checksum), None).await, "{algorithm}");
            let wrong = Checksum::new(algorithm, "00".repeat(algorithm.digest_len()));
            assert!(!file_ok(&path, Some(&wrong), None).await, "{algorithm}");
        }
        assert!(file_ok(&path, None, None).await);
        assert!(file_ok(&path, None, Some(3)).await);
        assert!(!file_ok(&path, None, Some(4)).await);
        assert!(!file_ok(dir.join("missing"), None, None).await);

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn hashing_streams_the_whole_file() {
        let dir = std::env::temp_dir().join(format!("dmcl-hash-big-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("big.bin");
        let data = vec![7u8; 1024 * 1024 + 3];
        tokio::fs::write(&path, &data).await.unwrap();

        for algorithm in HashAlgorithm::ALL {
            assert_eq!(
                hash_file(&path, algorithm).await.unwrap(),
                hash_bytes(algorithm, &data)
            );
        }
        assert!(Checksum::sha256(hash_bytes(HashAlgorithm::Sha256, &data))
            .verify_file(&path)
            .await
            .unwrap());

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
