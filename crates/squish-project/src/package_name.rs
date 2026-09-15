//! 包名领域类型。 / Package-name domain type.

use std::{borrow::Borrow, fmt, ops::Deref};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 无效的 xmlsquish 包名。 / An invalid xmlsquish package name.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("package name must be a non-empty sequence of ASCII letters, digits, '-' or '_'")]
pub struct InvalidPackageName;

/// 经过清单领域规则验证的稳定包名。 / Stable package name validated by manifest-domain rules.
///
/// This is the single package-name grammar shared by manifest validation, project creation,
/// manager requests, and CLI parsing. Keeping the invariant in a type prevents those entry
/// points from gradually accepting different names.
///
/// # Examples
///
/// ```
/// use squish_project::PackageName;
///
/// let name = PackageName::new("support_agent")?;
/// assert_eq!(name.as_str(), "support_agent");
/// # Ok::<(), squish_project::InvalidPackageName>(())
/// ```
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct PackageName(String);

/// 根据清单的唯一包名文法验证文本。 / Validates text with the manifest's single package-name grammar.
///
/// # Errors
///
/// Returns [`InvalidPackageName`] for an empty name or for any byte outside ASCII letters,
/// digits, `-`, and `_`. This function never normalizes the input.
pub fn validate_package_name(value: &str) -> Result<(), InvalidPackageName> {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(InvalidPackageName)
    }
}

impl PackageName {
    /// 验证并创建包名。 / Validates and constructs a package name.
    ///
    /// # Errors
    ///
    /// Empty names and names containing anything other than ASCII letters, digits, `-`, or `_`
    /// are rejected. No spelling is normalized or silently rewritten.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidPackageName> {
        let value = value.into();
        validate_package_name(&value)?;
        Ok(Self(value))
    }

    /// 判断文本是否符合共享包名文法。 / Tests text against the shared package-name grammar.
    #[must_use]
    pub fn is_valid(value: &str) -> bool {
        validate_package_name(value).is_ok()
    }

    /// 返回已验证的文本。 / Returns the validated text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 消耗类型并返回底层文本。 / Consumes the type and returns its text.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for PackageName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for PackageName {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Deref for PackageName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for PackageName {
    type Error = InvalidPackageName;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for PackageName {
    type Error = InvalidPackageName;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PackageName> for String {
    fn from(value: PackageName) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exact_manifest_grammar_without_normalizing() {
        for valid in ["a", "Agent42", "support-agent", "support_agent", "0"] {
            let name = PackageName::new(valid).unwrap();
            assert_eq!(name.as_str(), valid);
        }
    }

    #[test]
    fn rejects_empty_unicode_whitespace_and_punctuation() {
        for invalid in ["", "a b", "agent.", "代理", "a/b", "a\\b", "a:b"] {
            assert_eq!(PackageName::new(invalid), Err(InvalidPackageName));
        }
    }

    #[test]
    fn serde_round_trip_revalidates_input() {
        let name = PackageName::new("Agent_42").unwrap();
        let encoded = serde_json::to_string(&name).unwrap();
        assert_eq!(encoded, "\"Agent_42\"");
        assert_eq!(serde_json::from_str::<PackageName>(&encoded).unwrap(), name);
        assert!(serde_json::from_str::<PackageName>("\"not valid\"").is_err());
    }
}
