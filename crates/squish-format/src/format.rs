use std::{fmt, str::FromStr};

use crate::{Diagnostic, LosslessXml, Span};

/// 决定稳定格式规则的样式版本。 / Style edition selecting stable formatting rules.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum StyleEdition {
    /// 第一版：属性前一个空格、等号周围及标签闭合前无空格。
    /// / Edition one: one space before attributes, none around `=` or tag closure.
    #[default]
    V1,
}

impl fmt::Display for StyleEdition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V1 => formatter.write_str("1"),
        }
    }
}

/// 未知样式版本。 / Unknown style edition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownStyleEdition(String);

impl fmt::Display for UnknownStyleEdition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported formatting style edition {:?}",
            self.0
        )
    }
}
impl std::error::Error for UnknownStyleEdition {}

impl FromStr for StyleEdition {
    type Err = UnknownStyleEdition;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "1" => Ok(Self::V1),
            _ => Err(UnknownStyleEdition(value.to_owned())),
        }
    }
}

/// 对可证明为标签内 trivia 的单次替换。 / One replacement proven to target intra-tag trivia.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatEdit {
    /// 原文件范围。 / Range in the original file.
    pub span: Span,
    /// 该范围必须仍包含的字节。 / Bytes the range must still contain.
    pub expected: Vec<u8>,
    /// 替换字节。 / Replacement bytes.
    pub replacement: Vec<u8>,
}

/// 不执行 I/O 的确定性格式计划，可供 `--check`、diff 和事务提交复用。
/// / Deterministic, I/O-free format plan shared by `--check`, diff, and transactional publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatPlan {
    original: Vec<u8>,
    edits: Vec<FormatEdit>,
}

impl FormatPlan {
    /// 计划是否会改变文件。 / Whether the plan changes the file.
    pub fn is_changed(&self) -> bool {
        !self.edits.is_empty()
    }
    /// 按源码顺序返回编辑，可直接生成 diff。 / Returns source-ordered edits suitable for diff generation.
    pub fn edits(&self) -> &[FormatEdit] {
        &self.edits
    }
    /// 在内存中应用计划；上层 `FileTransaction` 负责真正写盘。
    /// / Applies the plan in memory; an upstream `FileTransaction` owns actual publication.
    pub fn apply(&self, source: &[u8]) -> Result<Vec<u8>, FormatPlanError> {
        if source != self.original {
            return Err(FormatPlanError::SourceChanged);
        }
        let mut output = Vec::with_capacity(source.len());
        let mut cursor = 0;
        for edit in &self.edits {
            if edit.span.start < cursor
                || edit.span.end > source.len()
                || source[edit.span.range()] != edit.expected
            {
                return Err(FormatPlanError::SourceChanged);
            }
            output.extend_from_slice(&source[cursor..edit.span.start]);
            output.extend_from_slice(&edit.replacement);
            cursor = edit.span.end;
        }
        output.extend_from_slice(&source[cursor..]);
        Ok(output)
    }
}

/// 格式计划无法安全应用。 / A format plan cannot be applied safely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatPlanError {
    /// 输入与生成计划时的内容不同。 / Input differs from the content used to create the plan.
    SourceChanged,
}
impl fmt::Display for FormatPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("source changed after the format plan was created")
    }
}
impl std::error::Error for FormatPlanError {}

/// 解析并规划 XML 格式化；非法或不完整输入只返回诊断，不产生可提交计划。
/// / Parses and plans XML formatting; invalid or incomplete input yields only a diagnostic.
pub fn format(source: &[u8], edition: StyleEdition) -> Result<FormatPlan, Diagnostic> {
    let xml = LosslessXml::parse(source)?;
    Ok(plan(&xml, edition))
}

fn plan(xml: &LosslessXml<'_>, edition: StyleEdition) -> FormatPlan {
    let mut edits = Vec::new();
    for tag in &xml.tags {
        for (span, separator) in tag
            .trivia
            .iter()
            .copied()
            .zip(tag.separators.iter().copied())
        {
            let replacement: &[u8] = match (edition, separator) {
                (StyleEdition::V1, true) => b" ",
                (StyleEdition::V1, false) => b"",
            };
            if xml.source()[span.range()] != *replacement {
                edits.push(FormatEdit {
                    span,
                    expected: xml.source()[span.range()].to_vec(),
                    replacement: replacement.to_vec(),
                });
            }
        }
    }
    edits.sort_by_key(|edit| edit.span.start);
    FormatPlan {
        original: xml.source().to_vec(),
        edits,
    }
}
