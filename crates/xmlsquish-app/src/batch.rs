use std::path::{Path, PathBuf};

use crate::{
    BatchReport, FailedFile, FileFailure, FileReport, FileStore, ProcessingStage, Squasher,
    TokenCounter, output_path_for,
};

/// Coordinates discovery, transformation, token measurement, and persistence.
pub struct BatchProcessor<F, S, T> {
    files: F,
    squasher: S,
    tokens: T,
}

impl<F, S, T> BatchProcessor<F, S, T>
where
    F: FileStore,
    S: Squasher,
    T: TokenCounter,
{
    pub fn new(files: F, squasher: S, tokens: T) -> Self {
        Self {
            files,
            squasher,
            tokens,
        }
    }

    /// Processes every discovered document and preserves per-file failures.
    ///
    /// Failures are accumulated so one corrupt document does not prevent
    /// unrelated documents from being processed.
    pub fn run(&self, paths: &[PathBuf]) -> BatchReport {
        let mut batch = BatchReport::default();

        for path in paths {
            match self.process_one(path) {
                Ok(file) => {
                    batch.stats.include(&file);
                    batch.files.push(file);
                }
                Err((stage, error)) => batch.failures.push(FailedFile {
                    path: path.clone(),
                    stage,
                    error,
                }),
            }
        }

        batch
    }

    fn process_one(&self, input_path: &Path) -> Result<FileReport, (ProcessingStage, FileFailure)> {
        let input = self
            .files
            .read(input_path)
            .map_err(|error| (ProcessingStage::Read, error))?;
        let squished = self
            .squasher
            .squish(&input)
            .map_err(|error| (ProcessingStage::Squish, error))?;
        let input_tokens = self
            .tokens
            .count(&input)
            .map_err(|error| (ProcessingStage::CountInputTokens, error))?;
        let output_tokens = self
            .tokens
            .count(&squished.text)
            .map_err(|error| (ProcessingStage::CountOutputTokens, error))?;
        let output_path = output_path_for(input_path);

        self.files
            .write(&output_path, &squished.text)
            .map_err(|error| (ProcessingStage::Write, error))?;

        Ok(FileReport {
            input_path: input_path.to_path_buf(),
            output_path,
            input_tokens,
            output_tokens,
            input_characters: char_count(&input),
            output_characters: char_count(&squished.text),
            recognized_whitespace: squished.recognized_whitespace,
            removed_whitespace: squished.removed_whitespace,
            inserted_whitespace: squished.inserted_whitespace,
        })
    }
}

fn char_count(text: &str) -> u64 {
    u64::try_from(text.chars().count()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::SquishResult;

    struct MemoryFiles {
        documents: HashMap<PathBuf, String>,
        writes: RefCell<HashMap<PathBuf, String>>,
        read_failure: Option<PathBuf>,
    }

    impl FileStore for MemoryFiles {
        fn read(&self, path: &Path) -> Result<String, FileFailure> {
            if self.read_failure.as_deref() == Some(path) {
                return Err(FileFailure::new("cannot read"));
            }
            self.documents
                .get(path)
                .cloned()
                .ok_or_else(|| FileFailure::new("missing"))
        }

        fn write(&self, path: &Path, contents: &str) -> Result<(), FileFailure> {
            self.writes
                .borrow_mut()
                .insert(path.to_path_buf(), contents.to_owned());
            Ok(())
        }
    }

    struct CollapseWhitespace;

    impl Squasher for CollapseWhitespace {
        fn squish(&self, input: &str) -> Result<SquishResult, FileFailure> {
            let recognized = input
                .chars()
                .filter(|character| matches!(character, ' ' | '\t' | '\n' | '\r'))
                .count();
            let text = input.split_whitespace().collect::<Vec<_>>().join(" ");
            let retained = text.matches(' ').count();
            Ok(SquishResult::new(
                text,
                recognized as u64,
                recognized.saturating_sub(retained) as u64,
                0,
            ))
        }
    }

    struct UnicodeScalarTokens;

    impl TokenCounter for UnicodeScalarTokens {
        fn count(&self, text: &str) -> Result<u64, FileFailure> {
            Ok(text.chars().count() as u64)
        }
    }

    #[test]
    fn processes_documents_and_aggregates_only_written_files() {
        let files = MemoryFiles {
            documents: HashMap::from([
                (PathBuf::from("a.xml"), "<a>  x </a>".into()),
                (PathBuf::from("b.xml"), "<b>\n y\t</b>".into()),
            ]),
            writes: RefCell::default(),
            read_failure: None,
        };
        let processor = BatchProcessor::new(files, CollapseWhitespace, UnicodeScalarTokens);

        let report = processor.run(&[PathBuf::from("a.xml"), PathBuf::from("b.xml")]);

        assert!(report.is_success());
        assert_eq!(report.stats.processed_files, 2);
        assert_eq!(report.stats.input_characters, 22);
        assert_eq!(report.stats.output_characters, 20);
        assert_eq!(report.stats.recognized_whitespace, 6);
        assert_eq!(report.stats.removed_whitespace, 2);
        assert_eq!(
            report.stats.input_characters - report.stats.removed_whitespace
                + report.stats.inserted_whitespace,
            report.stats.output_characters
        );
        assert_eq!(report.files[0].output_path, PathBuf::from("a.o.xml"));
    }

    #[test]
    fn keeps_processing_after_a_file_failure() {
        let files = MemoryFiles {
            documents: HashMap::from([
                (PathBuf::from("bad.xml"), "bad".into()),
                (PathBuf::from("good.xml"), "good".into()),
            ]),
            writes: RefCell::default(),
            read_failure: Some(PathBuf::from("bad.xml")),
        };
        let processor = BatchProcessor::new(files, CollapseWhitespace, UnicodeScalarTokens);

        let report = processor.run(&[PathBuf::from("bad.xml"), PathBuf::from("good.xml")]);

        assert_eq!(report.stats.processed_files, 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].stage, ProcessingStage::Read);
        assert_eq!(report.failures[0].path, PathBuf::from("bad.xml"));
    }

    #[test]
    fn unicode_character_counts_are_not_byte_counts() {
        let files = MemoryFiles {
            documents: HashMap::from([(PathBuf::from("中文.xml"), "<a>萌</a>".into())]),
            writes: RefCell::default(),
            read_failure: None,
        };
        let processor = BatchProcessor::new(files, CollapseWhitespace, UnicodeScalarTokens);

        let report = processor.run(&[PathBuf::from("中文.xml")]);

        assert_eq!(report.stats.input_characters, 8);
        assert_eq!(report.stats.output_characters, 8);
    }

    #[test]
    fn inserted_whitespace_participates_in_character_identity() {
        struct InsertSeparator;

        impl Squasher for InsertSeparator {
            fn squish(&self, _input: &str) -> Result<SquishResult, FileFailure> {
                Ok(SquishResult::new("<a> </a>".into(), 0, 0, 1))
            }
        }

        let files = MemoryFiles {
            documents: HashMap::from([(PathBuf::from("a.xml"), "<a></a>".into())]),
            writes: RefCell::default(),
            read_failure: None,
        };
        let processor = BatchProcessor::new(files, InsertSeparator, UnicodeScalarTokens);

        let report = processor.run(&[PathBuf::from("a.xml")]);
        let stats = report.stats;

        assert_eq!(stats.inserted_whitespace, 1);
        assert_eq!(
            stats.input_characters - stats.removed_whitespace + stats.inserted_whitespace,
            stats.output_characters
        );
    }
}
