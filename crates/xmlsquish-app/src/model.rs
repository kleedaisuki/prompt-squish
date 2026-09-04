use std::fmt;
use std::path::{Path, PathBuf};

/// The text and whitespace measurements produced by the squashing engine.
///
/// `recognized_whitespace` counts XML `S` characters in the complete source,
/// including markup. `removed_whitespace` counts source whitespace characters
/// that do not survive the transformation; `inserted_whitespace` counts spaces
/// added between atoms that were adjacent in the source. Consequently, a
/// whitespace-only transformation obeys
/// `output characters = input characters - removed + inserted`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SquishResult {
    pub text: String,
    pub recognized_whitespace: u64,
    pub removed_whitespace: u64,
    pub inserted_whitespace: u64,
}

impl SquishResult {
    pub fn new(
        text: String,
        recognized_whitespace: u64,
        removed_whitespace: u64,
        inserted_whitespace: u64,
    ) -> Self {
        Self {
            text,
            recognized_whitespace,
            removed_whitespace,
            inserted_whitespace,
        }
    }
}

/// Aggregate measurements for all successfully written files.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BatchStats {
    pub processed_files: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub input_characters: u64,
    pub output_characters: u64,
    pub recognized_whitespace: u64,
    pub removed_whitespace: u64,
    pub inserted_whitespace: u64,
}

impl BatchStats {
    /// Token reduction as a fraction of the input token count.
    ///
    /// Empty input has no meaningful compression ratio, so it returns `None`.
    pub fn compression_ratio(self) -> Option<f64> {
        if self.input_tokens == 0 {
            return None;
        }

        let saved = self.input_tokens as i128 - self.output_tokens as i128;
        Some(saved as f64 / self.input_tokens as f64)
    }

    pub fn compression_percent(self) -> Option<f64> {
        self.compression_ratio().map(|ratio| ratio * 100.0)
    }

    pub(crate) fn include(&mut self, report: &FileReport) {
        self.processed_files = self.processed_files.saturating_add(1);
        self.input_tokens = self.input_tokens.saturating_add(report.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(report.output_tokens);
        self.input_characters = self
            .input_characters
            .saturating_add(report.input_characters);
        self.output_characters = self
            .output_characters
            .saturating_add(report.output_characters);
        self.recognized_whitespace = self
            .recognized_whitespace
            .saturating_add(report.recognized_whitespace);
        self.removed_whitespace = self
            .removed_whitespace
            .saturating_add(report.removed_whitespace);
        self.inserted_whitespace = self
            .inserted_whitespace
            .saturating_add(report.inserted_whitespace);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileReport {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub input_characters: u64,
    pub output_characters: u64,
    pub recognized_whitespace: u64,
    pub removed_whitespace: u64,
    pub inserted_whitespace: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessingStage {
    Read,
    Squish,
    CountInputTokens,
    CountOutputTokens,
    Write,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileFailure {
    message: String,
}

impl FileFailure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for FileFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FileFailure {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailedFile {
    pub path: PathBuf,
    pub stage: ProcessingStage,
    pub error: FileFailure,
}

/// Complete observable outcome of a batch.
///
/// One bad file does not hide successful work on independent files. Discovery
/// errors are returned by [`crate::BatchProcessor::run`] because there is no
/// well-defined batch to process in that case; per-file failures are recorded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatchReport {
    pub stats: BatchStats,
    pub files: Vec<FileReport>,
    pub failures: Vec<FailedFile>,
}

impl BatchReport {
    pub fn is_success(&self) -> bool {
        self.failures.is_empty()
    }

    pub fn attempted_files(&self) -> usize {
        self.files.len().saturating_add(self.failures.len())
    }

    pub fn successful_files(&self) -> usize {
        self.files.len()
    }

    pub fn failed_files(&self) -> usize {
        self.failures.len()
    }
}

/// Derives `name.o.xml` beside an input XML file without assuming UTF-8 paths.
pub fn output_path_for(input: &Path) -> PathBuf {
    let mut name = input
        .file_stem()
        .map_or_else(Default::default, |stem| stem.to_os_string());
    name.push(".o.xml");
    input.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_a_sibling_with_o_xml_suffix() {
        assert_eq!(
            output_path_for(Path::new("prompts/system.xml")),
            PathBuf::from("prompts/system.o.xml")
        );
    }

    #[test]
    fn compression_rate_handles_empty_and_expanding_output() {
        assert_eq!(BatchStats::default().compression_ratio(), None);
        let stats = BatchStats {
            input_tokens: 2,
            output_tokens: 3,
            ..BatchStats::default()
        };
        assert_eq!(stats.compression_ratio(), Some(-0.5));
    }
}
