//! Opt-in project-local JSONL tracing with no dependency on product generation.
//!
//! Each invocation owns a new file, so concurrent processes never interleave records.
//! Trace write failures are reported separately and never change command semantics.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use squish_cli::TraceMode;
use squish_kernel::{EventSink, SinkError};
use squish_manager::ProjectBuildLayout;
use squish_protocol::{Event, InvocationId};

/// Shared recording state; absent state is the allocation-free disabled path.
#[derive(Clone, Default)]
pub(crate) struct Trace(Option<Arc<Recording>>);

/// A single command's monotonic clock and serialized output stream.
struct Recording {
    id: InvocationId,
    mode: TraceMode,
    start: Instant,
    output: Mutex<Output>,
    layout: ProjectBuildLayout,
    root: PathBuf,
    spool: PathBuf,
}

/// Buffered file plus the first failure, retained for a single final warning.
struct Output {
    writer: BufWriter<File>,
    error: Option<String>,
}

impl Trace {
    /// Creates a new trace in the project metadata namespace, never overwriting a run.
    pub(crate) fn open(
        root: &Path,
        layout: ProjectBuildLayout,
        mode: TraceMode,
        id: InvocationId,
    ) -> io::Result<Self> {
        if mode == TraceMode::Off {
            return Ok(Self::default());
        }
        let root = fs::canonicalize(root)?;
        let directory = root.join(".temp/xmlsquish-traces");
        create_safe_directory(&root, &directory)?;
        let spool = directory.join(format!("{}.jsonl", id.as_str()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&spool)?;
        Ok(Self(Some(Arc::new(Recording {
            id,
            mode,
            start: Instant::now(),
            layout,
            root,
            spool,
            output: Mutex::new(Output {
                writer: BufWriter::new(file),
                error: None,
            }),
        }))))
    }

    /// Shares command identity with the protocol stream.
    pub(crate) fn invocation(&self) -> Option<InvocationId> {
        self.0.as_ref().map(|recording| recording.id.clone())
    }

    /// Writes one versioned record; failures disable further writes without failing builds.
    fn record(&self, kind: &str, payload: Value) {
        let Some(recording) = &self.0 else {
            return;
        };
        let mut output = recording.output.lock().unwrap_or_else(|p| p.into_inner());
        if output.error.is_some() {
            return;
        }
        let record = json!({
            "schema": "xmlsquish.trace.v1",
            "invocation": recording.id.as_str(),
            "kind": kind,
            "elapsed_us": recording.start.elapsed().as_micros(),
            "data": payload,
        });
        let result = serde_json::to_writer(&mut output.writer, &record)
            .map_err(io::Error::other)
            .and_then(|()| output.writer.write_all(b"\n"));
        if let Err(error) = result {
            output.error = Some(error.to_string());
        }
    }

    /// Records only command identity, never arguments, environment values, or product bytes.
    pub(crate) fn begin(&self, operation: squish_protocol::OperationKind) {
        if self.0.is_none() {
            return;
        }
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        self.record(
            "command_start",
            json!({"operation": format!("{operation:?}"), "unix_ms": unix_ms,
            "version": env!("CARGO_PKG_VERSION")}),
        );
    }

    /// Opens a named bootstrap or dispatch span; completion is recorded on every return path.
    pub(crate) fn span(&self, name: &'static str) -> Span {
        if self.0.is_some() {
            self.record("span_start", json!({"name": name}));
        }
        Span {
            trace: self.clone(),
            name,
            start: self.0.as_ref().map(|_| Instant::now()),
        }
    }

    /// Records kernel lifecycles only in events mode; these retain action and planning timings.
    pub(crate) fn event(&self, event: &Event) {
        if self.0.as_ref().is_some_and(|r| r.mode == TraceMode::Events) {
            self.record("event", json!({"event": event}));
        }
    }

    /// Records the actual process outcome and flushes all buffered records across runs.
    pub(crate) fn finish(&self, code: u8) -> Result<(), String> {
        let Some(recording) = &self.0 else {
            return Ok(());
        };
        self.record("command_finish", json!({"exit_code": code,
            "status": if code == 0 { "success" } else if code == 130 { "cancelled" } else { "failure" }}));
        let mut output = recording.output.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(error) = output.writer.flush() {
            output.error.get_or_insert_with(|| error.to_string());
        }
        if let Some(error) = &output.error {
            return Err(error.clone());
        }
        recording
            .layout
            .validate_existing_aliases()
            .map_err(|error| error.to_string())?;
        let directory = recording.layout.metadata_root().join("traces");
        create_safe_directory(&recording.root, &directory).map_err(|error| error.to_string())?;
        let destination = directory.join(format!("{}.jsonl", recording.id.as_str()));
        // create_new excludes replacement of existing records and aliases; copying also supports
        // target directories residing on another volume. The spool stays recoverable on failure.
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| error.to_string())?;
        let mut source = File::open(&recording.spool).map_err(|error| error.to_string())?;
        io::copy(&mut source, &mut target).map_err(|error| error.to_string())?;
        target.flush().map_err(|error| error.to_string())?;
        drop(source);
        fs::remove_file(&recording.spool).map_err(|error| error.to_string())?;
        Ok(())
    }
}

/// Creates each component without accepting symlinks or Windows reparse points.
/// This mirrors the project layout trust boundary; it is not a hostile concurrent-FS sandbox.
fn create_safe_directory(root: &Path, directory: &Path) -> io::Result<()> {
    let relative = directory.strip_prefix(root).map_err(io::Error::other)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::create_dir(&current) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(&current)?;
        #[cfg(windows)]
        let alias = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let alias = metadata.file_type().is_symlink();
        if alias || !metadata.is_dir() {
            return Err(io::Error::other(
                "trace directory must not contain filesystem aliases",
            ));
        }
    }
    Ok(())
}

/// RAII timing span: returned errors and early exits still close the span.
pub(crate) struct Span {
    trace: Trace,
    name: &'static str,
    start: Option<Instant>,
}

impl Drop for Span {
    fn drop(&mut self) {
        if self.trace.0.is_some() {
            self.trace.record(
                "span_finish",
                json!({"name": self.name,
                "duration_us": self.start.map(|start| start.elapsed().as_micros()).unwrap_or(0)}),
            );
        }
    }
}

/// An observational tee: telemetry cannot make a renderer or build fail.
pub(crate) struct TracingSink {
    /// The existing authoritative presentation sink.
    pub(crate) sink: Arc<dyn EventSink>,
    /// Optional recorder shared with the command boundary.
    pub(crate) trace: Trace,
}

impl EventSink for TracingSink {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        self.trace.event(&event);
        self.sink.emit(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test files remain inside the repository's scratch namespace.
    fn fixture() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp/trace-tests");
        fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }

    #[test]
    fn disabled_tracing_creates_no_files() {
        let root = fixture();
        let trace = Trace::open(
            root.path(),
            ProjectBuildLayout::project_local_for_tests(root.path()),
            TraceMode::Off,
            InvocationId::new("off").unwrap(),
        )
        .unwrap();
        trace.begin(squish_protocol::OperationKind::Build);
        drop(trace.span("bootstrap"));
        trace.finish(1).unwrap();
        assert!(!root.path().join("target").exists());
    }

    #[test]
    fn successive_runs_preserve_failure_outcomes_and_spans() {
        let root = fixture();
        for (id, code) in [("first", 1), ("second", 0)] {
            let trace = Trace::open(
                root.path(),
                ProjectBuildLayout::project_local_for_tests(root.path()),
                TraceMode::Summary,
                InvocationId::new(id).unwrap(),
            )
            .unwrap();
            trace.begin(squish_protocol::OperationKind::Build);
            drop(trace.span("compose"));
            trace.finish(code).unwrap();
            let bytes = fs::read_to_string(
                root.path()
                    .join(format!("target/xmlsquish/metadata/traces/{id}.jsonl")),
            )
            .unwrap();
            let records: Vec<Value> = bytes
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(records.len(), 4);
            assert_eq!(records[3]["data"]["exit_code"], code);
            assert_eq!(records[2]["kind"], "span_finish");
            assert!(records.iter().all(|r| r["invocation"] == id));
        }
        assert_eq!(
            fs::read_dir(root.path().join("target/xmlsquish/metadata/traces"))
                .unwrap()
                .count(),
            2
        );
    }
    #[test]
    fn events_are_selective_and_keep_the_command_identity() {
        let root = fixture();
        let event = Event::new(
            InvocationId::new("events").unwrap(),
            0,
            squish_protocol::EventPayload::PlanningStarted {
                job: squish_protocol::JobId::new("build").unwrap(),
                attempt: squish_protocol::PlanningAttemptId::new("plan").unwrap(),
            },
        );
        for mode in [TraceMode::Summary, TraceMode::Events] {
            let id = if mode == TraceMode::Events {
                "events"
            } else {
                "summary"
            };
            let trace = Trace::open(
                root.path(),
                ProjectBuildLayout::project_local_for_tests(root.path()),
                mode,
                InvocationId::new(id).unwrap(),
            )
            .unwrap();
            trace.event(&event);
            trace.finish(0).unwrap();
            let bytes = fs::read_to_string(
                root.path()
                    .join(format!("target/xmlsquish/metadata/traces/{id}.jsonl")),
            )
            .unwrap();
            assert_eq!(
                bytes.contains("planning_started"),
                mode == TraceMode::Events
            );
            assert_eq!(trace.invocation().unwrap().as_str(), id);
        }
    }

    #[test]
    fn custom_target_layout_and_migration_preserve_the_active_trace() {
        let root = fixture();
        let canonical = fs::canonicalize(root.path()).unwrap();
        let layout = ProjectBuildLayout::new(canonical, "build/state").unwrap();
        let trace = Trace::open(
            root.path(),
            layout.clone(),
            TraceMode::Summary,
            InvocationId::new("custom").unwrap(),
        )
        .unwrap();
        trace.begin(squish_protocol::OperationKind::Build);
        fs::create_dir_all(layout.metadata_root()).unwrap();
        fs::remove_dir_all(layout.ownership_root()).unwrap();
        trace.finish(1).unwrap();
        assert!(layout.metadata_root().join("traces/custom.jsonl").is_file());
        assert!(!root.path().join("target").exists());
    }

    #[cfg(unix)]
    #[test]
    fn trace_publication_rejects_metadata_aliases() {
        let root = fixture();
        let external = fixture();
        let layout = ProjectBuildLayout::project_local_for_tests(root.path());
        let trace = Trace::open(
            root.path(),
            layout.clone(),
            TraceMode::Summary,
            InvocationId::new("alias").unwrap(),
        )
        .unwrap();
        fs::create_dir_all(layout.ownership_root()).unwrap();
        std::os::unix::fs::symlink(external.path(), layout.metadata_root()).unwrap();
        assert!(trace.finish(0).is_err());
        assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
    }
}
