//! 内部宏。

/// 声明一个"若干已知取值 + `Other(String)` 兜底"的字符串枚举。
///
/// 兜底变体是必需的：这些取值来自 Minecraft 自己的文档，而我们会**把它们写回去**，
/// 因此遇到不认识的取值必须原样保留，否则会损坏版本 JSON。
macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum $name {
            $(
                $variant,
            )+
            /// 本 crate 不认识的取值，原样保留。
            Other(String),
        }

        impl $name {
            /// 该变体在文档里的字符串形式。
            pub fn as_str(&self) -> &str {
                match self {
                    $($name::$variant => $text,)+
                    $name::Other(raw) => raw,
                }
            }
        }

        impl From<String> for $name {
            fn from(raw: String) -> Self {
                match raw.as_str() {
                    $($text => $name::$variant,)+
                    _ => $name::Other(raw),
                }
            }
        }

        impl From<&str> for $name {
            fn from(raw: &str) -> Self {
                $name::from(raw.to_string())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                let raw: String = serde::Deserialize::deserialize(deserializer)?;
                Ok(raw.into())
            }
        }
    };
}

pub(crate) use string_enum;
