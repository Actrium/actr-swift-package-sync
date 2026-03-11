use std::fmt::{self, Display};
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use thiserror::Error;

// Protobuf Package name
// 语法要求：由字母、数字和下划线组成，各部分用 . 分隔（类似 Java 包名）
// 命名规范：
// - 全小写
// - 建议采用反向域名形式（如 com.example.project）避免冲突
// - 不能以数字开头，不能包含除 . 和 _ 外的特殊字符

// Protobuf Service name
// 语法要求：由字母、数字和下划线组成
// 命名规范：
// - 采用 PascalCase（帕斯卡命名法，首字母大写）
// - 通常使用名词或名词短语
// - 不能以数字开头，不能包含特殊字符

// Protobuf Method name
// 语法要求：由字母、数字和下划线组成
// 命名规范：
// - 采用 camelCase（驼峰命名法，首字母小写）
// - 通常使用动词或动词短语
// - 不能以数字开头，不能包含特殊字符

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct PackageName(String);

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ServiceName(String);

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct MethodName(String);

/// A validated actor name.
///
/// Names must:
/// - Start with an alphabetic character
/// - End with an alphanumeric character
/// - Contain only alphanumeric characters, hyphens, underscores, and dots
/// - Be non-empty
/// - Not exceed 32 characters in length
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Name(String);

#[derive(Debug, Error, Eq, PartialEq)]
pub enum NameError {
    #[error("Name is empty")]
    Empty,

    #[error("Name exceeds 32 characters, length: {0}")]
    TooLong(usize),

    #[error("Name must start with an alphabetic character, found: {0}")]
    InvalidStartChar(char),

    #[error("Name must end with an alphanumeric character, found: {0}")]
    InvalidEndChar(char),

    #[error("Name contains invalid character: {0}")]
    InvalidChar(char),
}

impl Name {
    /// Creates a new validated Name.
    ///
    /// # Errors
    ///
    /// Returns `NameError` with specific reason if the name doesn't meet the validation criteria.
    pub fn new(name: String) -> Result<Self, NameError> {
        if name.is_empty() {
            return Err(NameError::Empty);
        }
        if name.len() > 32 {
            return Err(NameError::TooLong(name.len()));
        }
        let mut chars = name.chars();
        let first = chars.next().ok_or(NameError::Empty)?;
        if !first.is_alphabetic() {
            return Err(NameError::InvalidStartChar(first));
        }
        let mut last = first;
        for c in chars {
            last = c;
            if !c.is_alphanumeric() && c != '-' && c != '_' && c != '.' {
                return Err(NameError::InvalidChar(c));
            }
        }
        if !last.is_alphanumeric() {
            return Err(NameError::InvalidEndChar(last));
        }
        Ok(Self(name))
    }
}

impl FromStr for Name {
    type Err = NameError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s.to_string())
    }
}

impl Deref for Name {
    type Target = String;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Name {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TryFrom<String> for Name {
    type Error = NameError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ----------------------------- PackageName -----------------------------

#[derive(Debug, Error, Eq, PartialEq)]
pub enum PackageNameError {
    #[error("Package name is empty")]
    Empty,

    #[error("Package name exceeds 256 characters, length: {0}")]
    TooLong(usize),

    #[error("Package name must not start with a digit or '.', found: {0}")]
    InvalidStartChar(char),

    #[error("Package name must not end with '.', found: {0}")]
    InvalidEndChar(char),

    #[error("Package name contains invalid character: {0}")]
    InvalidChar(char),

    #[error("Package name contains empty segment")]
    EmptySegment,

    #[error("Package name segment must not start with a digit, found: {0}")]
    InvalidSegmentStartChar(char),
}

impl PackageName {
    pub fn new(name: String) -> Result<Self, PackageNameError> {
        const MAX_LEN: usize = 256;
        if name.is_empty() {
            return Err(PackageNameError::Empty);
        }
        if name.len() > MAX_LEN {
            return Err(PackageNameError::TooLong(name.len()));
        }

        let mut chars = name.chars();
        let first = chars.next().ok_or(PackageNameError::Empty)?;

        // First character rules: cannot be digit or '.'; letters must be lowercase
        if first == '.' || first.is_ascii_digit() {
            return Err(PackageNameError::InvalidStartChar(first));
        }
        if first.is_alphabetic() && first.is_uppercase() {
            return Err(PackageNameError::InvalidChar(first));
        }
        if !(first.is_alphanumeric() || first == '_') {
            // Only letters/digits/underscore are allowed ('.' only as separator handled below)
            return Err(PackageNameError::InvalidChar(first));
        }

        let mut last = first;
        let mut at_segment_start = false; // after first char we've already started first segment

        for c in chars {
            // Dot separates segments
            if c == '.' {
                if last == '.' {
                    return Err(PackageNameError::EmptySegment);
                }
                at_segment_start = true;
                last = c;
                continue;
            }

            // Segment start cannot be digit
            if at_segment_start && c.is_ascii_digit() {
                return Err(PackageNameError::InvalidSegmentStartChar(c));
            }
            at_segment_start = false;

            // Allowed chars inside segments
            if !(c.is_alphanumeric() || c == '_') {
                return Err(PackageNameError::InvalidChar(c));
            }
            // Enforce lowercase for alphabetic
            if c.is_alphabetic() && c.is_uppercase() {
                return Err(PackageNameError::InvalidChar(c));
            }

            last = c;
        }

        if last == '.' {
            return Err(PackageNameError::InvalidEndChar(last));
        }

        Ok(Self(name))
    }
}

impl FromStr for PackageName {
    type Err = PackageNameError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s.to_string())
    }
}

impl Deref for PackageName {
    type Target = String;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for PackageName {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TryFrom<String> for PackageName {
    type Error = PackageNameError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ----------------------------- ServiceName -----------------------------

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ServiceNameError {
    #[error("Service name is empty")]
    Empty,

    #[error("Service name exceeds 64 characters, length: {0}")]
    TooLong(usize),

    #[error("Service name must start with an uppercase alphabetic character, found: {0}")]
    InvalidStartChar(char),

    #[error("Service name must end with an alphanumeric character, found: {0}")]
    InvalidEndChar(char),

    #[error("Service name contains invalid character: {0}")]
    InvalidChar(char),
}

impl ServiceName {
    pub fn new(name: String) -> Result<Self, ServiceNameError> {
        const MAX_LEN: usize = 64;
        if name.is_empty() {
            return Err(ServiceNameError::Empty);
        }
        if name.len() > MAX_LEN {
            return Err(ServiceNameError::TooLong(name.len()));
        }

        let mut chars = name.chars();
        let first = chars.next().ok_or(ServiceNameError::Empty)?;
        if !first.is_alphabetic() || !first.is_uppercase() {
            return Err(ServiceNameError::InvalidStartChar(first));
        }
        let mut last = first;
        for c in chars {
            if !(c.is_alphanumeric() || c == '_') {
                return Err(ServiceNameError::InvalidChar(c));
            }
            last = c;
        }
        if !last.is_alphanumeric() {
            return Err(ServiceNameError::InvalidEndChar(last));
        }
        Ok(Self(name))
    }
}

impl FromStr for ServiceName {
    type Err = ServiceNameError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s.to_string())
    }
}

impl Deref for ServiceName {
    type Target = String;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ServiceName {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TryFrom<String> for ServiceName {
    type Error = ServiceNameError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl Display for ServiceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ----------------------------- MethodName -----------------------------

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MethodNameError {
    #[error("Method name is empty")]
    Empty,

    #[error("Method name exceeds 64 characters, length: {0}")]
    TooLong(usize),

    #[error("Method name must start with a lowercase alphabetic character, found: {0}")]
    InvalidStartChar(char),

    #[error("Method name must end with an alphanumeric character, found: {0}")]
    InvalidEndChar(char),

    #[error("Method name contains invalid character: {0}")]
    InvalidChar(char),
}

impl MethodName {
    pub fn new(name: String) -> Result<Self, MethodNameError> {
        const MAX_LEN: usize = 64;
        if name.is_empty() {
            return Err(MethodNameError::Empty);
        }
        if name.len() > MAX_LEN {
            return Err(MethodNameError::TooLong(name.len()));
        }

        let mut chars = name.chars();
        let first = chars.next().ok_or(MethodNameError::Empty)?;
        if !first.is_alphabetic() || !first.is_lowercase() {
            return Err(MethodNameError::InvalidStartChar(first));
        }
        let mut last = first;
        for c in chars {
            if !(c.is_alphanumeric() || c == '_') {
                return Err(MethodNameError::InvalidChar(c));
            }
            last = c;
        }
        if !last.is_alphanumeric() {
            return Err(MethodNameError::InvalidEndChar(last));
        }
        Ok(Self(name))
    }
}

impl FromStr for MethodName {
    type Err = MethodNameError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s.to_string())
    }
}

impl Deref for MethodName {
    type Target = String;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for MethodName {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TryFrom<String> for MethodName {
    type Error = MethodNameError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl Display for MethodName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_name() {
        assert!(Name::new("valid-name_123".to_string()).is_ok());
        assert!(Name::new("A".to_string()).is_ok());
        assert!(Name::new("actor-1".to_string()).is_ok());
        assert!(Name::new("actor.1".to_string()).is_ok());
        assert!(Name::new("com.example.actor".to_string()).is_ok());
    }

    #[test]
    fn test_invalid_name() {
        assert_eq!(Name::new("".to_string()).unwrap_err(), NameError::Empty);
        assert_eq!(
            Name::new("-invalid".to_string()).unwrap_err(),
            NameError::InvalidStartChar('-')
        );
        assert_eq!(
            Name::new(".invalid".to_string()).unwrap_err(),
            NameError::InvalidStartChar('.')
        );
        assert_eq!(
            Name::new("invalid-".to_string()).unwrap_err(),
            NameError::InvalidEndChar('-')
        );
        assert_eq!(
            Name::new("invalid.".to_string()).unwrap_err(),
            NameError::InvalidEndChar('.')
        );
        assert_eq!(
            Name::new("has invalid char!".to_string()).unwrap_err(),
            NameError::InvalidChar(' ')
        );
        assert_eq!(
            Name::new("too_long_name_that_exceeds_thirty_two_characters".to_string()).unwrap_err(),
            NameError::TooLong(48)
        );
        assert_eq!(
            Name::new("1starts_with_number".to_string()).unwrap_err(),
            NameError::InvalidStartChar('1')
        );
    }

    #[test]
    fn test_valid_package_name() {
        assert!(PackageName::new("com.example.project".to_string()).is_ok());
        assert!(PackageName::new("a_b.c_d.e1".to_string()).is_ok());
        assert!(PackageName::new("example".to_string()).is_ok());
        assert!(PackageName::new("_internal.pkg".to_string()).is_ok());
    }

    #[test]
    fn test_invalid_package_name() {
        assert_eq!(
            PackageName::new("".to_string()).unwrap_err(),
            PackageNameError::Empty
        );
        assert_eq!(
            PackageName::new(".leading".to_string()).unwrap_err(),
            PackageNameError::InvalidStartChar('.')
        );
        assert_eq!(
            PackageName::new("trailing.".to_string()).unwrap_err(),
            PackageNameError::InvalidEndChar('.')
        );
        assert_eq!(
            PackageName::new("com..example".to_string()).unwrap_err(),
            PackageNameError::EmptySegment
        );
        assert_eq!(
            PackageName::new("com.1example".to_string()).unwrap_err(),
            PackageNameError::InvalidSegmentStartChar('1')
        );
        assert_eq!(
            PackageName::new("Com.Example".to_string()).unwrap_err(),
            PackageNameError::InvalidChar('C')
        );
        assert_eq!(
            PackageName::new("com.exa$mple".to_string()).unwrap_err(),
            PackageNameError::InvalidChar('$')
        );
    }

    #[test]
    fn test_valid_service_name() {
        assert!(ServiceName::new("Echo".to_string()).is_ok());
        assert!(ServiceName::new("UserService".to_string()).is_ok());
        assert!(ServiceName::new("HTTPV1".to_string()).is_ok());
        assert!(ServiceName::new("Service_Name".to_string()).is_ok());
    }

    #[test]
    fn test_invalid_service_name() {
        assert_eq!(
            ServiceName::new("".to_string()).unwrap_err(),
            ServiceNameError::Empty
        );
        assert_eq!(
            ServiceName::new("service".to_string()).unwrap_err(),
            ServiceNameError::InvalidStartChar('s')
        );
        assert_eq!(
            ServiceName::new("_Service".to_string()).unwrap_err(),
            ServiceNameError::InvalidStartChar('_')
        );
        assert_eq!(
            ServiceName::new("Service-".to_string()).unwrap_err(),
            ServiceNameError::InvalidChar('-')
        );
        assert_eq!(
            ServiceName::new("Service_".to_string()).unwrap_err(),
            ServiceNameError::InvalidEndChar('_')
        );
    }

    #[test]
    fn test_valid_method_name() {
        assert!(MethodName::new("echo".to_string()).is_ok());
        assert!(MethodName::new("doWork".to_string()).is_ok());
        assert!(MethodName::new("get_v1".to_string()).is_ok());
    }

    #[test]
    fn test_invalid_method_name() {
        assert_eq!(
            MethodName::new("".to_string()).unwrap_err(),
            MethodNameError::Empty
        );
        assert_eq!(
            MethodName::new("1call".to_string()).unwrap_err(),
            MethodNameError::InvalidStartChar('1')
        );
        assert_eq!(
            MethodName::new("Call".to_string()).unwrap_err(),
            MethodNameError::InvalidStartChar('C')
        );
        assert_eq!(
            MethodName::new("do-".to_string()).unwrap_err(),
            MethodNameError::InvalidChar('-')
        );
        assert_eq!(
            MethodName::new("do_".to_string()).unwrap_err(),
            MethodNameError::InvalidEndChar('_')
        );
    }
}
