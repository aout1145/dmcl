//! 只读解析 jar 内 class 的常量池。

use std::io::Read;

pub(crate) type Zip = zip::ZipArchive<std::fs::File>;

/// 读取 jar 内某个 class 的常量池字符串。
pub(crate) fn class_strings(archive: &mut Zip, entry: &str) -> Option<Vec<String>> {
    let mut entry = archive.by_name(entry).ok()?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    constant_pool_strings(&bytes)
}

/// 只收集 class 常量池里的 UTF-8 字符串。
pub(crate) fn constant_pool_strings(bytes: &[u8]) -> Option<Vec<String>> {
    let mut cursor = ByteCursor::new(bytes);
    if cursor.u4()? != 0xCAFE_BABE {
        return None;
    }
    cursor.skip(4)?; // minor_version + major_version
    let count = cursor.u2()? as usize;

    let mut strings = Vec::new();
    let mut index = 1;
    while index < count {
        match cursor.u1()? {
            1 => {
                let length = cursor.u2()? as usize;
                let raw = cursor.take(length)?;
                strings.push(String::from_utf8_lossy(raw).into_owned());
            }
            3 | 4 => cursor.skip(4)?,
            // Long / Double 占两个常量池槽位。
            5 | 6 => {
                cursor.skip(8)?;
                index += 1;
            }
            7 | 8 | 16 | 19 | 20 => cursor.skip(2)?,
            9 | 10 | 11 | 12 | 17 | 18 => cursor.skip(4)?,
            15 => cursor.skip(3)?,
            _ => return None,
        }
        index += 1;
    }
    Some(strings)
}

struct ByteCursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> ByteCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let end = self.position.checked_add(length)?;
        let slice = self.bytes.get(self.position..end)?;
        self.position = end;
        Some(slice)
    }

    fn skip(&mut self, length: usize) -> Option<()> {
        self.take(length).map(|_| ())
    }

    fn u1(&mut self) -> Option<u8> {
        Some(*self.take(1)?.first()?)
    }

    fn u2(&mut self) -> Option<u16> {
        let bytes = self.take(2)?;
        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u4(&mut self) -> Option<u32> {
        let bytes = self.take(4)?;
        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(count: u16) -> Vec<u8> {
        let mut bytes = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 52];
        bytes.extend_from_slice(&count.to_be_bytes());
        bytes
    }

    #[test]
    fn utf8_constants_are_collected_and_wide_constants_skip_two_slots() {
        let mut bytes = header(5);
        bytes.push(1);
        bytes.extend_from_slice(&2u16.to_be_bytes());
        bytes.extend_from_slice(b"ok");
        // Long 占两个槽位，后面紧接下一个字符串。
        bytes.push(5);
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.push(1);
        bytes.extend_from_slice(&6u16.to_be_bytes());
        bytes.extend_from_slice(b"1.20.1");

        assert_eq!(constant_pool_strings(&bytes).unwrap(), ["ok", "1.20.1"]);
    }

    #[test]
    fn invalid_or_truncated_classes_are_rejected() {
        assert!(constant_pool_strings(b"not a class").is_none());

        let mut truncated = header(2);
        truncated.push(1);
        truncated.extend_from_slice(&10u16.to_be_bytes());
        assert!(constant_pool_strings(&truncated).is_none());
    }
}
