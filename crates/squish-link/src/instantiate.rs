//! ADR 0007 宏展开机。 / ADR 0007 macro-expansion machine.

use crate::{
    ArchiveDirective, InstantiateError, InstantiationFailure, InstantiationFrame, LinkedProgram,
};
use regex::Regex;
use squish_ir::{
    BindingRef, DefAddr, DocumentItem, DocumentItemId, DocumentRegion, DocumentRegionId,
    EntityKind, ExpansionTrace, FileBinding, FrameId, FrameIdentity, FrameRecord, LinkedDocumentIr,
    LinkedOpRef, LocalName, Op, OriginNode, QualifiedOriginRef, RelocatableUnitIr, ScalarExpr,
    ScalarValueId, SequenceValueId, StringId, SubstitutionKind, SubstitutionStep, TraceRef,
    Validate,
};
use std::{
    borrow::{Borrow, Cow},
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    hash::{Hash, Hasher},
    ops::{Deref, Range},
    rc::Rc,
    sync::Arc,
};

/// 一次实例化的显式资源上限。 / Explicit resource limits for one instantiation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Budgets {
    /// 包含 entry 帧的最大展开深度。 / Maximum expansion depth including the entry frame.
    pub max_depth: u64,
    /// 包含 entry 的最大帧数。 / Maximum frame count including the entry.
    pub max_expansions: u64,
    /// 任一求值序列的完整后端中立语义负载上限。 / Maximum complete backend-neutral semantic payload of any evaluated sequence.
    ///
    /// 该量包括事件、扩展名、属性和值的 UTF-8 字节及固定宽度结构开销；它不是某个后端的最终产品大小。
    /// This measure includes events, expanded names, attributes, values, and fixed structural
    /// overhead; it is not any backend's final product size.
    pub max_output_bytes: u64,
}
impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_depth: 1_024,
            max_expansions: 100_000,
            max_output_bytes: 64 * 1_024 * 1_024,
        }
    }
}

/// 实例化结果；文档与 trace 是不可分割的联合证据。
/// Instantiation result; document and trace form one inseparable evidence bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstantiateOutput {
    /// 完全展开的后端中立文档。 / Fully expanded backend-neutral document.
    pub document: LinkedDocumentIr,
    /// 与文档 occurrence 一一对应的动态来源。 / Dynamic provenance joined to document occurrences.
    pub trace: ExpansionTrace,
    /// Ordered archive requests retaining the defining source through macro expansion.
    pub directives: Vec<ArchiveDirective>,
}

/// 无状态 ADR 0007 求值器；绝不生成 XML 字符串。 / Stateless ADR 0007 evaluator; never emits XML strings.
#[derive(Clone, Copy, Debug, Default)]
pub struct Instantiator;

impl Instantiator {
    /// 以冻结参数和预算实例化 entry。 / Instantiates the entry with frozen arguments and budgets.
    pub fn instantiate(
        &self,
        program: &LinkedProgram,
        args: BTreeMap<LocalName, String>,
        budgets: Budgets,
    ) -> Result<InstantiateOutput, InstantiateError> {
        Machine::new(program, args, budgets)?.run()
    }

    /// Instantiates with actual failure-frame provenance for structured diagnostic rendering.
    ///
    /// Successful output and budget/error semantics are identical to `instantiate`. Only an
    /// error copies the selected parent-chain call/declaration origins while the machine still
    /// owns its frames. This avoids interpreting frame IDs after their provenance was dropped.
    ///
    /// ```ignore
    /// let failure = Instantiator.instantiate_with_diagnostics(&program, args, budgets)
    ///     .expect_err("the recursive fixture exhausts its budget");
    /// let primary = failure.error.origin.as_ref()
    ///     .and_then(|origin| program.resolve_origin(origin));
    /// ```
    pub fn instantiate_with_diagnostics(
        &self,
        program: &LinkedProgram,
        args: BTreeMap<LocalName, String>,
        budgets: Budgets,
    ) -> Result<InstantiateOutput, InstantiationFailure> {
        let mut machine = Machine::new(program, args, budgets)?;
        match machine.run() {
            Ok(output) => Ok(output),
            Err(error) => Err(machine.failure_context(error)),
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Counts actual adopted runtime String payloads, not Arc clones or heap allocator events.
    static OWNED_TEXT_MATERIALIZATIONS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Test-thread witness for String-backed text construction, without resetting parallel tests.
#[cfg(test)]
pub(crate) fn owned_text_materializations() -> (usize, usize) {
    OWNED_TEXT_MATERIALIZATIONS.with(std::cell::Cell::get)
}

/// Text ownership, not a global literal cache: unit strings already belong to the program.
#[derive(Clone)]
enum TextBacking {
    /// Runtime concatenations, external arguments and existing shared scalar fact strings.
    Owned(Arc<String>),
    /// A literal view retains its exact immutable unit, never a duplicated whole input.
    Unit {
        unit: Arc<RelocatableUnitIr>,
        string: StringId,
    },
}

impl TextBacking {
    /// Returns the immutable full backing; unit IDs were checked by the sole constructor.
    fn as_str(&self) -> &str {
        match self {
            Self::Owned(text) => text.as_str(),
            Self::Unit { unit, string } => &unit.header().semantic_strings[string.0 as usize],
        }
    }
}

/// Immutable visible UTF-8 text retaining its owner only for the invocation lifetime.
///
/// Occurrences own distinct provenance, not necessarily distinct bytes. Equality and hashing
/// use the visible substring; backing identity/range never selects a scalar ID.
#[derive(Clone)]
struct Text {
    /// Immutable string storage; partial views never mutate or trim this shared allocation.
    backing: TextBacking,
    /// Checked byte boundaries into the backing, always relative to its UTF-8 representation.
    range: Range<usize>,
}

impl fmt::Debug for Text {
    /// Diagnostics expose only the visible value, not unrelated input bytes or entire unit IR.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Text")
            .field("visible", &self.as_str())
            .field("range", &self.range)
            .finish()
    }
}

impl Text {
    /// Creates a view only when both bounds describe a valid UTF-8 substring.
    fn view(backing: Arc<String>, range: Range<usize>) -> Option<Self> {
        backing.get(range.clone())?;
        Some(Self {
            backing: TextBacking::Owned(backing),
            range,
        })
    }

    /// Borrows a checked literal from the unit already retained by the executable program.
    fn unit_view(
        unit: Arc<RelocatableUnitIr>,
        string: StringId,
        range: Range<usize>,
    ) -> Option<Self> {
        unit.header()
            .semantic_strings
            .get(string.0 as usize)?
            .get(range.clone())?;
        Some(Self {
            backing: TextBacking::Unit { unit, string },
            range,
        })
    }

    /// Test-only owned-storage witness; unit-backed values intentionally have no String Arc.
    #[cfg(test)]
    fn owned_backing(&self) -> Option<&Arc<String>> {
        match &self.backing {
            TextBacking::Owned(text) => Some(text),
            TextBacking::Unit { .. } => None,
        }
    }

    /// Returns precisely the visible bytes, never unrelated bytes retained in the backing.
    fn as_str(&self) -> &str {
        &self.backing.as_str()[self.range.clone()]
    }

    /// Transfers a uniquely owned full string, or copies only the visible substring.
    fn into_owned(self) -> String {
        let Self { backing, range } = self;
        match backing {
            TextBacking::Owned(text) if range.start == 0 && range.end == text.len() => {
                Arc::try_unwrap(text).unwrap_or_else(|text| text.as_str().to_owned())
            }
            backing => backing.as_str()[range].to_owned(),
        }
    }

    /// Slices relative visible bytes without copying captured/fact text.
    fn slice(&self, range: Range<usize>) -> Option<Self> {
        self.as_str().get(range.clone())?;
        let start = self.range.start.checked_add(range.start)?;
        let end = self.range.start.checked_add(range.end)?;
        Some(Self {
            backing: self.backing.clone(),
            range: start..end,
        })
    }
}

impl PartialEq for Text {
    /// Equal visible bytes are equal keys even when their backings/ranges differ.
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl Eq for Text {}
impl Hash for Text {
    /// Matches the `str` hash exactly so borrowed hash-map lookup remains valid.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}
impl Borrow<str> for Text {
    /// Borrows the same visible key used by Eq/Hash, never the complete backing.
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl From<String> for Text {
    /// Transfers a full owned string; no additional payload copy occurs.
    fn from(value: String) -> Self {
        #[cfg(test)]
        OWNED_TEXT_MATERIALIZATIONS.with(|count| {
            let (calls, bytes) = count.get();
            count.set((calls + 1, bytes + value.len()));
        });
        let range = 0..value.len();
        Self {
            backing: TextBacking::Owned(Arc::new(value)),
            range,
        }
    }
}

impl From<&str> for Text {
    /// Owns a borrowed literal at its existing materialization boundary.
    fn from(value: &str) -> Self {
        value.to_owned().into()
    }
}

impl Deref for Text {
    type Target = str;

    /// Makes all scalar consumers observe the same visible UTF-8 value.
    fn deref(&self) -> &str {
        self.as_str()
    }
}

/// Scalar materialization state; only multiple nonempty views require contiguous new bytes.
#[derive(Default)]
enum ScalarText {
    /// No occurrence was evaluated; finishing produces the ordinary empty scalar.
    #[default]
    Empty,
    /// One visible value, possibly preceded/followed by empty provenance contributions.
    View(Text),
    /// Concatenation owns only visible bytes, never an entire unrelated partial-view backing.
    Joined(String),
}

impl ScalarText {
    /// Appends visible bytes while retaining single-fragment ownership without a copy.
    fn append(&mut self, value: Text) {
        if value.is_empty() {
            if matches!(self, Self::Empty) {
                *self = Self::View(value);
            }
            return;
        }
        *self = match std::mem::take(self) {
            Self::Empty => Self::View(value),
            Self::View(first) if first.is_empty() => Self::View(value),
            Self::View(first) => {
                let mut text = String::with_capacity(first.len().saturating_add(value.len()));
                text.push_str(first.as_str());
                text.push_str(value.as_str());
                Self::Joined(text)
            }
            Self::Joined(mut text) => {
                text.push_str(value.as_str());
                Self::Joined(text)
            }
        };
    }

    /// Publishes immutable scalar bytes once; joined strings transfer their owned buffers.
    fn finish(self) -> Text {
        match self {
            Self::Empty => String::new().into(),
            Self::View(text) => text,
            Self::Joined(text) => text.into(),
        }
    }
}

#[derive(Clone, Debug)]
struct Value {
    text: Text,
    substitutions: Vec<SubstitutionStep>,
}

#[derive(Clone, Debug)]
struct Env {
    frame: FrameId,
    args: Rc<BTreeMap<String, Value>>,
    slots: Rc<BTreeMap<String, SlotValue>>,
    captures: Rc<BTreeMap<String, Value>>,
}

#[derive(Clone, Debug)]
struct Occurrence {
    kind: OccurrenceKind,
    trace: TraceRef,
    sequence: Option<SequenceValueId>,
}
#[derive(Clone, Debug)]
struct SlotValue {
    occurrences: Vec<Occurrence>,
    sequence: SequenceValueId,
}
#[derive(Clone, Debug)]
enum OccurrenceKind {
    Text(Text),
    Comment(Text),
    Pi(Text, Text),
    Element {
        name: squish_ir::ExpandedName,
        attributes: Vec<(squish_ir::ExpandedName, Text)>,
        children: Vec<Occurrence>,
    },
}

#[derive(Clone, Debug)]
struct CallState<'a> {
    target: DefAddr,
    op_ref: LinkedOpRef,
    caller: Env,
    args: &'a [squish_ir::Argument],
    fills: &'a [squish_ir::Fill],
    values: BTreeMap<String, Value>,
    slots: BTreeMap<String, SlotValue>,
    output: usize,
}

#[derive(Clone, Debug)]
enum Task<'a> {
    Region {
        at: squish_ir::LinkedRegionRef,
        env: Env,
        output: usize,
    },
    Operation {
        at: LinkedOpRef,
        env: Env,
        output: usize,
    },
    ElementDone {
        at: LinkedOpRef,
        env: Env,
        name: squish_ir::ExpandedName,
        attrs: Vec<(squish_ir::ExpandedName, Text)>,
        children: usize,
        output: usize,
    },
    Arg {
        call: CallState<'a>,
        index: usize,
    },
    ArgDone {
        call: CallState<'a>,
        index: usize,
        buffer: usize,
    },
    Fill {
        call: CallState<'a>,
        index: usize,
    },
    FillDone {
        call: CallState<'a>,
        index: usize,
        buffer: usize,
    },
}

/// Maximum scalar arena size scanned directly before indexing the immutable prefix.
///
/// Small invocations commonly reuse one long argument. Hashing and duplicating that entire
/// key on every lookup costs more than a bounded equality check. The cap is independent of
/// workload size: large distinct-value workloads still enter the indexed path once, rather
/// than restoring an unbounded quadratic scan. Eight is a conservative fixed small-domain
/// cutoff, not a claimed universal hardware-optimal crossover.
const LINEAR_SCALAR_LIMIT: usize = 8;

/// Preserves first-insertion scalar IDs without paying for an index in tiny value domains.
#[derive(Default)]
struct ScalarIndex {
    /// Created once after the bounded prefix; keys are session state, never trace ordering.
    large: Option<HashMap<Text, u32>>,
}

impl ScalarIndex {
    /// Finds the first arena occurrence, promoting the complete prefix at most once.
    fn find(&mut self, values: &[Text], value: &str) -> Option<u32> {
        if values.len() <= LINEAR_SCALAR_LIMIT && self.large.is_none() {
            return values
                .iter()
                .position(|old| old.as_str() == value)
                .map(|id| id as u32);
        }
        let index = self.large.get_or_insert_with(|| {
            let mut index = HashMap::with_capacity(values.len());
            for (id, text) in values.iter().enumerate() {
                index.entry(text.clone()).or_insert(id as u32);
            }
            index
        });
        index.get(value).copied()
    }

    /// Appends a new value once; indexed mode never derives an ID from map iteration.
    fn intern(&mut self, values: &mut Vec<Text>, value: &Text) -> u32 {
        if let Some(id) = self.find(values, value.as_str()) {
            return id;
        }
        let id = values.len() as u32;
        values.push(value.clone());
        if let Some(index) = &mut self.large {
            index.insert(value.clone(), id);
        }
        id
    }
}

struct Machine<'a> {
    program: &'a LinkedProgram,
    budgets: Budgets,
    tasks: Vec<Task<'a>>,
    buffers: Vec<Vec<Occurrence>>,
    buffer_bytes: Vec<u64>,
    frames: Vec<FrameRecord>,
    frame_origins: Vec<QualifiedOriginRef>,
    scalar_values: Vec<Text>,
    /// Session-only bounded small-pool lookup and lazy index; never serialized.
    scalar_index: ScalarIndex,
    sequences: Vec<Vec<DocumentItemId>>,
    origins: Vec<OriginNode>,
    directives: Vec<ArchiveDirective>,
    directive_bytes: u64,
}

impl<'a> Machine<'a> {
    fn new(
        program: &'a LinkedProgram,
        args: BTreeMap<String, String>,
        budgets: Budgets,
    ) -> Result<Self, InstantiateError> {
        if budgets.max_depth == 0 || budgets.max_expansions == 0 {
            return Err(InstantiateError::new(
                "RUN001",
                "root frame exceeds depth or expansion budget",
            ));
        }
        let expected: BTreeSet<_> = program
            .image()
            .entry
            .required_params
            .iter()
            .cloned()
            .collect();
        let actual: BTreeSet<_> = args.keys().cloned().collect();
        if expected != actual {
            return Err(InstantiateError::new(
                "RUN002",
                "entry arguments do not match its signature",
            ));
        }
        let root = program.image().entry.root_region;
        let definition_origin = origin_for_region(program, root)
            .ok_or_else(|| InstantiateError::new("RUN003", "entry root has no source origin"))?;
        let mut scalar_values = Vec::new();
        let mut scalar_index = ScalarIndex::default();
        let mut frame_args = Vec::with_capacity(args.len());
        let values: BTreeMap<_, _> = args
            .into_iter()
            .map(|(name, text)| {
                // Entry arguments retain their original arena positions, including duplicate
                // values. Frame IDs always name the first equal value, as in macro interning.
                let first = scalar_index.find(&scalar_values, &text);
                let id = first.unwrap_or(scalar_values.len() as u32);
                let text: Text = match first {
                    Some(id) => scalar_values[id as usize].clone(),
                    None => text.into(),
                };
                frame_args.push((name.clone(), ScalarValueId(id)));
                if let Some(index) = &mut scalar_index.large {
                    index.entry(text.clone()).or_insert(id);
                }
                scalar_values.push(text.clone());
                (
                    name,
                    Value {
                        text,
                        substitutions: Vec::new(),
                    },
                )
            })
            .collect();
        let env = Env {
            frame: FrameId(0),
            args: Rc::new(values),
            slots: Rc::default(),
            captures: Rc::default(),
        };
        let frames = vec![FrameRecord {
            id: FrameId(0),
            parent: None,
            identity: FrameIdentity::Entry,
            call_origin: None,
            definition_origin: definition_origin.clone(),
            depth: 1,
            args: frame_args.clone(),
            fills: Vec::new(),
        }];
        let origins = frame_args
            .into_iter()
            .map(|(name, value)| OriginNode::ExternalArgument { name, value })
            .collect();
        Ok(Self {
            program,
            budgets,
            tasks: vec![Task::Region {
                at: root,
                env,
                output: 0,
            }],
            buffers: vec![Vec::new()],
            buffer_bytes: vec![0],
            frames,
            frame_origins: vec![definition_origin],
            scalar_values,
            scalar_index,
            sequences: Vec::new(),
            origins,
            directives: Vec::new(),
            directive_bytes: 0,
        })
    }

    fn run(&mut self) -> Result<InstantiateOutput, InstantiateError> {
        while let Some(task) = self.tasks.pop() {
            match task {
                Task::Region { at, env, output } => {
                    let region = region(self.program, at)?;
                    for op in region.ops.iter().rev() {
                        self.tasks.push(Task::Operation {
                            at: LinkedOpRef {
                                unit_slot: at.unit_slot,
                                op: *op,
                            },
                            env: env.clone(),
                            output,
                        });
                    }
                }
                Task::Operation { at, env, output } => self.operation(at, env, output)?,
                Task::ElementDone {
                    at,
                    env,
                    name,
                    attrs,
                    children,
                    output,
                } => {
                    let children = std::mem::take(&mut self.buffers[children]);
                    let trace = self.trace_ref(at, &env, Vec::new())?;
                    self.emit(
                        output,
                        Occurrence {
                            kind: OccurrenceKind::Element {
                                name,
                                attributes: attrs,
                                children,
                            },
                            trace,
                            sequence: None,
                        },
                    )?;
                }
                Task::Arg { mut call, index } => {
                    let arguments = call.args;
                    let Some(arg) = arguments.get(index) else {
                        self.tasks.push(Task::Fill { call, index: 0 });
                        continue;
                    };
                    if call.values.contains_key(&arg.name) {
                        return Err(self.failure(
                            "RUN004",
                            "duplicate argument",
                            call.caller.frame,
                            Some(call.op_ref),
                        ));
                    }
                    match &arg.value {
                        ScalarExpr::RenderText(region) => {
                            let region = *region;
                            let buffer = self.buffer();
                            let caller = call.caller.clone();
                            let unit_slot = call.op_ref.unit_slot;
                            self.tasks.push(Task::ArgDone {
                                call,
                                index,
                                buffer,
                            });
                            // Borrow the immutable fact through the external program lifetime,
                            // not through the mutable machine being used to emit occurrences.
                            let program = self.program;
                            let literal = program
                                .optimized(unit_slot)
                                .and_then(|unit| unit.static_scalars.get(&region));
                            if let Some(literal) = literal {
                                // Preserve each operation occurrence, trace and semantic budget
                                // while skipping task dispatch for compile-time literal bodies.
                                for segment in &literal.segments {
                                    let at = LinkedOpRef {
                                        unit_slot,
                                        op: segment.op,
                                    };
                                    self.emit_simple(
                                        at,
                                        &caller,
                                        buffer,
                                        OccurrenceKind::Text(
                                            Text::view(
                                                Arc::clone(&literal.text),
                                                segment.bytes.clone(),
                                            )
                                            .ok_or_else(|| {
                                                InstantiateError::new(
                                                    "RUN034",
                                                    "invalid static scalar UTF-8 range",
                                                )
                                            })?,
                                        ),
                                    )?;
                                }
                            } else {
                                self.tasks.push(Task::Region {
                                    at: squish_ir::LinkedRegionRef { unit_slot, region },
                                    env: caller,
                                    output: buffer,
                                });
                            }
                        }
                        value => {
                            let value = self.scalar(call.op_ref.unit_slot, value, &call.caller)?;
                            call.values.insert(arg.name.clone(), value);
                            self.tasks.push(Task::Arg {
                                call,
                                index: index + 1,
                            });
                        }
                    }
                }
                Task::ArgDone {
                    mut call,
                    index,
                    buffer,
                } => {
                    let occurrences = std::mem::take(&mut self.buffers[buffer]);
                    let mut text = ScalarText::default();
                    let mut substitutions = Vec::new();
                    for occurrence in occurrences {
                        let OccurrenceKind::Text(value) = occurrence.kind else {
                            return Err(self.failure(
                                "RUN005",
                                "scalar argument body produced a non-text node",
                                call.caller.frame,
                                Some(call.op_ref),
                            ));
                        };
                        // Text sharing never removes occurrence validation, substitutions or
                        // earlier budget charges. Only multiple nonempty pieces materialize.
                        text.append(value);
                        substitutions.extend(occurrence.trace.substitution_chain);
                    }
                    substitutions.push(SubstitutionStep {
                        kind: SubstitutionKind::ScalarBody,
                        origin: self.op_origin(call.op_ref)?,
                    });
                    call.values.insert(
                        call.args[index].name.clone(),
                        Value {
                            text: text.finish(),
                            substitutions,
                        },
                    );
                    self.tasks.push(Task::Arg {
                        call,
                        index: index + 1,
                    });
                }
                Task::Fill { call, index } => {
                    let fills = call.fills;
                    let Some(fill) = fills.get(index) else {
                        self.enter(call)?;
                        continue;
                    };
                    if call.slots.contains_key(&fill.name) {
                        return Err(self.failure(
                            "RUN006",
                            "duplicate fill",
                            call.caller.frame,
                            Some(call.op_ref),
                        ));
                    }
                    let buffer = self.buffer();
                    let caller = call.caller.clone();
                    let unit_slot = call.op_ref.unit_slot;
                    self.tasks.push(Task::FillDone {
                        call,
                        index,
                        buffer,
                    });
                    self.tasks.push(Task::Region {
                        at: squish_ir::LinkedRegionRef {
                            unit_slot,
                            region: fill.body,
                        },
                        env: caller,
                        output: buffer,
                    });
                }
                Task::FillDone {
                    mut call,
                    index,
                    buffer,
                } => {
                    let sequence = SequenceValueId(self.sequences.len() as u32);
                    self.sequences.push(Vec::new());
                    let mut occurrences = std::mem::take(&mut self.buffers[buffer]);
                    for occurrence in &mut occurrences {
                        occurrence.sequence = Some(sequence);
                    }
                    call.slots.insert(
                        call.fills[index].name.clone(),
                        SlotValue {
                            occurrences,
                            sequence,
                        },
                    );
                    self.tasks.push(Task::Fill {
                        call,
                        index: index + 1,
                    });
                }
            }
        }
        let occurrences = std::mem::take(&mut self.buffers[0]);
        let flattened = flatten(occurrences, self.program.image(), self.sequences.len())?;
        let mut document = flattened.document;
        let document_trace = flattened.traces;
        self.sequences = flattened.sequences;
        canonicalize_document(&mut document);
        // The lookup no longer participates after evaluation. Release its shared keys before
        // producing the public owned pool so uniquely held full values can transfer buffers.
        drop(std::mem::take(&mut self.scalar_index));
        let scalar_values = canonicalize_scalars(
            std::mem::take(&mut self.scalar_values),
            &mut self.frames,
            &mut self.origins,
        );
        let trace = ExpansionTrace {
            frames: std::mem::take(&mut self.frames),
            scalar_values,
            sequences: std::mem::take(&mut self.sequences),
            debug_strings: Vec::new(),
            origins: std::mem::take(&mut self.origins),
            document_items: document_trace,
        };
        document.validate().map_err(|e| {
            InstantiateError::new("RUN007", format!("invalid document produced: {e}"))
        })?;
        let root_kind = self.program.image().units
            [self.program.image().entry.root_region.unit_slot as usize]
            .kind;
        validate_document_shape(&document, root_kind)?;
        trace.validate_against_document(&document).map_err(|e| {
            InstantiateError::new("RUN008", format!("invalid expansion trace produced: {e}"))
        })?;
        Ok(InstantiateOutput {
            document,
            trace,
            directives: std::mem::take(&mut self.directives),
        })
    }

    fn operation(
        &mut self,
        at: LinkedOpRef,
        env: Env,
        output: usize,
    ) -> Result<(), InstantiateError> {
        let program = self.program;
        let op = operation(program, at)?;
        match op {
            Op::EmitText { value } => self.emit_simple(
                at,
                &env,
                output,
                OccurrenceKind::Text(self.literal_text(at.unit_slot, *value)?),
            )?,
            Op::EmitComment { value } => self.emit_simple(
                at,
                &env,
                output,
                OccurrenceKind::Comment(self.literal_text(at.unit_slot, *value)?),
            )?,
            Op::EmitPi { target, data } => self.emit_simple(
                at,
                &env,
                output,
                OccurrenceKind::Pi(
                    self.literal_text(at.unit_slot, *target)?,
                    self.literal_text(at.unit_slot, *data)?,
                ),
            )?,
            Op::EmitElement {
                name,
                attributes,
                children,
            } => {
                let name = self.qname(at.unit_slot, *name)?;
                let attrs = attributes
                    .iter()
                    .map(|a| {
                        Ok((
                            self.qname(at.unit_slot, a.name)?,
                            self.literal_text(at.unit_slot, a.value)?,
                        ))
                    })
                    .collect::<Result<_, InstantiateError>>()?;
                let buffer = self.buffer();
                self.tasks.push(Task::ElementDone {
                    at,
                    env: env.clone(),
                    name,
                    attrs,
                    children: buffer,
                    output,
                });
                self.tasks.push(Task::Region {
                    at: squish_ir::LinkedRegionRef {
                        unit_slot: at.unit_slot,
                        region: *children,
                    },
                    env,
                    output: buffer,
                });
            }
            Op::InsertScalar { value } => {
                let value = self.binding(at.unit_slot, value, &env)?;
                if !valid_xml_chars(&value.text) {
                    return Err(self.failure(
                        "RUN027",
                        "insert contains a character forbidden by XML 1.0",
                        env.frame,
                        Some(at),
                    ));
                }
                let mut substitutions = value.substitutions;
                substitutions.push(SubstitutionStep {
                    kind: SubstitutionKind::InsertScalar,
                    origin: self.op_origin(at)?,
                });
                let trace = self.trace_ref(at, &env, substitutions)?;
                self.emit(
                    output,
                    Occurrence {
                        kind: OccurrenceKind::Text(value.text),
                        trace,
                        sequence: None,
                    },
                )?;
            }
            Op::MatchRegex {
                input,
                pattern,
                captures,
                matched,
            } => {
                let input = match input {
                    squish_ir::MatchInput::Literal(id) => Value {
                        text: self.literal_text(at.unit_slot, *id)?,
                        substitutions: Vec::new(),
                    },
                    squish_ir::MatchInput::ReadBinding(b) => self.binding(at.unit_slot, b, &env)?,
                };
                let static_match = program
                    .optimized(at.unit_slot)
                    .and_then(|unit| unit.static_matches.get(&at.op));
                let found = match static_match {
                    Some(fact) if fact.matched => Some(Cow::Borrowed(&fact.captures)),
                    Some(_) => None,
                    None => {
                        let regex = self.regex(at.unit_slot, *pattern)?;
                        regex
                            .captures(&input.text)
                            .map(|found| {
                                captures
                                    .iter()
                                    .filter_map(|name| {
                                        found
                                            .name(name)
                                            .map(|value| (name.clone(), value.start()..value.end()))
                                    })
                                    .collect::<BTreeMap<_, _>>()
                            })
                            .map(Cow::Owned)
                    }
                };
                if let Some(found) = found {
                    let mut map = (*env.captures).clone();
                    let mut capture_records = Vec::new();
                    for name in captures {
                        if let Some(value) = found.get(name) {
                            let mut substitutions = input.substitutions.clone();
                            substitutions.push(SubstitutionStep {
                                kind: SubstitutionKind::ScalarBody,
                                origin: self.op_origin(at)?,
                            });
                            capture_records.push((name.clone(), value.start, value.end));
                            map.insert(
                                name.clone(),
                                Value {
                                    text: input.text.slice(value.clone()).ok_or_else(|| {
                                        InstantiateError::new(
                                            "RUN034",
                                            "invalid capture UTF-8 range",
                                        )
                                    })?,
                                    substitutions,
                                },
                            );
                        }
                    }
                    let input_node = squish_ir::OriginNodeId(self.origins.len() as u32);
                    self.origins.push(OriginNode::SourceSpan {
                        origin: self.op_origin(at)?,
                    });
                    for (capture, start, end) in capture_records {
                        self.origins.push(OriginNode::RegexCapture {
                            input: input_node,
                            capture,
                            matched_range: squish_ir::Span {
                                start: start as u64,
                                end: end as u64,
                            },
                        });
                    }
                    let branch = Env {
                        captures: Rc::new(map),
                        ..env
                    };
                    self.tasks.push(Task::Region {
                        at: squish_ir::LinkedRegionRef {
                            unit_slot: at.unit_slot,
                            region: *matched,
                        },
                        env: branch,
                        output,
                    });
                }
            }
            Op::Asset { path, name } => {
                let source = self.source(at.unit_slot)?;
                let path = self.string(at.unit_slot, *path)?;
                let name = self.string(at.unit_slot, *name)?;
                let trace = self.trace_ref(at, &env, Vec::new())?;
                self.directive_bytes = self
                    .directive_bytes
                    .saturating_add((path.len() + name.len()) as u64 + 64);
                if self.directive_bytes > self.budgets.max_output_bytes {
                    return Err(self.failure(
                        "RUN028",
                        "archive directives exceed output budget",
                        env.frame,
                        Some(at),
                    ));
                }
                self.directives.push(ArchiveDirective::Asset {
                    source,
                    path,
                    name,
                    trace,
                });
            }
            Op::Include { import, path, name } => {
                let source = self.source(at.unit_slot)?;
                let path = self.string(at.unit_slot, *path)?;
                let name = self.string(at.unit_slot, *name)?;
                let trace = self.trace_ref(at, &env, Vec::new())?;
                self.directive_bytes = self
                    .directive_bytes
                    .saturating_add((path.len() + name.len()) as u64 + 64);
                if self.directive_bytes > self.budgets.max_output_bytes {
                    return Err(self.failure(
                        "RUN028",
                        "archive directives exceed output budget",
                        env.frame,
                        Some(at),
                    ));
                }
                self.directives.push(ArchiveDirective::Include {
                    source,
                    import: *import,
                    path,
                    name,
                    trace,
                });
            }
            Op::ReadSlot { name } => {
                let Some(items) = env.slots.get(name) else {
                    return Ok(());
                };
                let origin = self.op_origin(at)?;
                for mut item in items.occurrences.clone() {
                    add_substitution(
                        &mut item,
                        SubstitutionStep {
                            kind: SubstitutionKind::SlotFill,
                            origin: origin.clone(),
                        },
                    );
                    self.emit(output, item)?;
                }
            }
            Op::Call {
                target: _,
                args,
                fills,
            } => {
                let target = self
                    .program
                    .image()
                    .link_map
                    .relocations
                    .binary_search_by_key(&at, |x| x.0)
                    .ok()
                    .map(|i| self.program.image().link_map.relocations[i].1)
                    .ok_or_else(|| {
                        self.failure(
                            "RUN010",
                            "call has no static relocation",
                            env.frame,
                            Some(at),
                        )
                    })?;
                self.tasks.push(Task::Arg {
                    call: CallState {
                        target,
                        op_ref: at,
                        caller: env,
                        args,
                        fills,
                        values: BTreeMap::new(),
                        slots: BTreeMap::new(),
                        output,
                    },
                    index: 0,
                });
            }
        }
        Ok(())
    }

    fn enter(&mut self, call: CallState<'a>) -> Result<(), InstantiateError> {
        let program = self.program;
        let def = program.definition(call.target).ok_or_else(|| {
            self.failure(
                "RUN011",
                "relocation target is missing",
                call.caller.frame,
                Some(call.op_ref),
            )
        })?;
        let depth = self.frames[call.caller.frame.0 as usize].depth + 1;
        if depth > self.budgets.max_depth {
            return Err(self.failure(
                "RUN012",
                "max-depth budget exceeded",
                call.caller.frame,
                Some(call.op_ref),
            ));
        }
        if self.frames.len() as u64 >= self.budgets.max_expansions {
            return Err(self.failure(
                "RUN013",
                "max-expansions budget exceeded",
                call.caller.frame,
                Some(call.op_ref),
            ));
        }
        let id = FrameId(self.frames.len() as u32);
        let args = call
            .values
            .iter()
            .map(|(n, v)| (n.clone(), ScalarValueId(self.intern_scalar(&v.text))))
            .collect();
        let fills = call
            .slots
            .iter()
            .map(|(n, value)| (n.clone(), value.sequence))
            .collect();
        let call_origin = self.op_origin(call.op_ref)?;
        let definition_origin =
            origin_for_definition(self.program, call.target).ok_or_else(|| {
                self.failure(
                    "RUN014",
                    "macro definition has no origin",
                    call.caller.frame,
                    Some(call.op_ref),
                )
            })?;
        self.frames.push(FrameRecord {
            id,
            parent: Some(call.caller.frame),
            identity: FrameIdentity::Macro(call.target),
            call_origin: Some(call_origin),
            definition_origin: definition_origin.clone(),
            depth,
            args,
            fills,
        });
        self.frame_origins.push(definition_origin);
        self.origins.push(OriginNode::Expansion {
            frame: id,
            producer: call.op_ref,
        });
        let env = Env {
            frame: id,
            args: Rc::new(call.values),
            slots: Rc::new(call.slots),
            captures: Rc::default(),
        };
        self.tasks.push(Task::Region {
            at: def.body,
            env,
            output: call.output,
        });
        Ok(())
    }

    fn scalar(&self, slot: u32, expr: &ScalarExpr, env: &Env) -> Result<Value, InstantiateError> {
        match expr {
            ScalarExpr::Literal(id) => Ok(Value {
                text: self.literal_text(slot, *id)?,
                substitutions: Vec::new(),
            }),
            ScalarExpr::ReadBinding(b) => self.binding(slot, b, env),
            ScalarExpr::RenderText(_) => unreachable!(),
        }
    }
    fn binding(
        &self,
        slot: u32,
        binding: &BindingRef,
        env: &Env,
    ) -> Result<Value, InstantiateError> {
        let value = match binding {
            BindingRef::Arg(name) => env.args.get(name),
            BindingRef::Match(name) => env.captures.get(name),
            BindingRef::File(kind) => {
                return Ok(Value {
                    text: file_binding(&self.source(slot)?, *kind)?.into(),
                    substitutions: Vec::new(),
                });
            }
        };
        value
            .cloned()
            .ok_or_else(|| self.failure("RUN015", "undefined binding", env.frame, None))
    }
    fn emit_simple(
        &mut self,
        at: LinkedOpRef,
        env: &Env,
        output: usize,
        kind: OccurrenceKind,
    ) -> Result<(), InstantiateError> {
        let trace = self.trace_ref(at, env, Vec::new())?;
        self.emit(
            output,
            Occurrence {
                kind,
                trace,
                sequence: None,
            },
        )
    }
    fn emit(&mut self, output: usize, item: Occurrence) -> Result<(), InstantiateError> {
        let producer = item.trace.producer_op;
        let frame = item.trace.frame;
        let bytes = self.buffer_bytes[output]
            .checked_add(item.cost())
            .ok_or_else(|| {
                self.failure(
                    "RUN016",
                    "max-output-bytes budget exceeded",
                    frame,
                    Some(producer),
                )
            })?;
        if bytes > self.budgets.max_output_bytes {
            return Err(self.failure(
                "RUN016",
                "max-output-bytes budget exceeded",
                frame,
                Some(producer),
            ));
        }
        self.buffer_bytes[output] = bytes;
        self.origins.push(OriginNode::SourceSpan {
            origin: self.op_origin(item.trace.producer_op)?,
        });
        self.buffers[output].push(item);
        Ok(())
    }
    fn trace_ref(
        &self,
        at: LinkedOpRef,
        env: &Env,
        substitutions: Vec<SubstitutionStep>,
    ) -> Result<TraceRef, InstantiateError> {
        Ok(TraceRef {
            producer_op: at,
            frame: env.frame,
            definition_origin: self.frame_origins[env.frame.0 as usize].clone(),
            call_origin: self.frames[env.frame.0 as usize].call_origin.clone(),
            substitution_chain: substitutions,
        })
    }
    fn buffer(&mut self) -> usize {
        self.buffers.push(Vec::new());
        self.buffer_bytes.push(0);
        self.buffers.len() - 1
    }
    fn intern_scalar(&mut self, value: &Text) -> u32 {
        self.scalar_index.intern(&mut self.scalar_values, value)
    }
    /// Retains a literal owner, not its payload copy; lookup errors match the String helper.
    fn literal_text(&self, slot: u32, id: StringId) -> Result<Text, InstantiateError> {
        let unit = self
            .program
            .unit_arc(slot)
            .ok_or_else(|| InstantiateError::new("RUN021", "unit slot is out of range"))?;
        let len = unit
            .header()
            .semantic_strings
            .get(id.0 as usize)
            .ok_or_else(|| InstantiateError::new("RUN017", "string ID is out of range"))?
            .len();
        Text::unit_view(unit, id, 0..len)
            .ok_or_else(|| InstantiateError::new("RUN034", "invalid literal UTF-8 range"))
    }

    fn string(&self, slot: u32, id: StringId) -> Result<String, InstantiateError> {
        let unit = self.unit(slot)?;
        unit.header()
            .semantic_strings
            .get(id.0 as usize)
            .cloned()
            .ok_or_else(|| InstantiateError::new("RUN017", "string ID is out of range"))
    }
    fn qname(
        &self,
        slot: u32,
        id: squish_ir::QNameId,
    ) -> Result<squish_ir::ExpandedName, InstantiateError> {
        self.unit(slot)?
            .header()
            .qnames
            .get(id.0 as usize)
            .cloned()
            .ok_or_else(|| InstantiateError::new("RUN018", "QName ID is out of range"))
    }
    fn regex(&self, slot: u32, id: squish_ir::RegexId) -> Result<&Regex, InstantiateError> {
        self.program
            .optimized(slot)
            .and_then(|unit| unit.regexes.get(id.0 as usize))
            .ok_or_else(|| InstantiateError::new("RUN019", "compiled regex ID is out of range"))
    }
    fn source(&self, slot: u32) -> Result<squish_ir::SourceKey, InstantiateError> {
        Ok(self.unit(slot)?.header().source.clone())
    }
    fn unit(&self, slot: u32) -> Result<&RelocatableUnitIr, InstantiateError> {
        self.program
            .unit(slot)
            .ok_or_else(|| InstantiateError::new("RUN021", "unit slot is out of range"))
    }
    fn op_origin(&self, at: LinkedOpRef) -> Result<QualifiedOriginRef, InstantiateError> {
        origin_for_op(self.program, at)
            .ok_or_else(|| InstantiateError::new("RUN022", "operation has no origin"))
    }
    fn failure(
        &self,
        code: &'static str,
        message: impl Into<String>,
        frame: FrameId,
        op: Option<LinkedOpRef>,
    ) -> InstantiateError {
        let mut chain = Vec::new();
        let mut cursor = Some(frame);
        while let Some(id) = cursor {
            chain.push(id);
            cursor = self.frames[id.0 as usize].parent;
        }
        chain.reverse();
        InstantiateError {
            code,
            message: message.into(),
            origin: op.and_then(|x| origin_for_op(self.program, x)),
            frame_chain: chain,
        }
    }

    /// Copies only witnessed failure-chain origins; successful execution does no extra work.
    fn failure_context(&self, error: InstantiateError) -> InstantiationFailure {
        let frames = error
            .frame_chain
            .iter()
            .filter_map(|id| self.frames.get(id.0 as usize))
            .map(|frame| InstantiationFrame {
                id: frame.id,
                identity: frame.identity.clone(),
                call_origin: frame.call_origin.clone(),
                definition_origin: frame.definition_origin.clone(),
            })
            .collect();
        InstantiationFailure { error, frames }
    }
}

impl Occurrence {
    fn cost(&self) -> u64 {
        let mut total = 0u64;
        let mut pending = vec![self];
        while let Some(item) = pending.pop() {
            total = total.saturating_add(match &item.kind {
                OccurrenceKind::Text(v) => 9 + v.len() as u64,
                OccurrenceKind::Comment(v) => 9 + v.len() as u64,
                OccurrenceKind::Pi(a, b) => 17 + a.len() as u64 + b.len() as u64,
                OccurrenceKind::Element {
                    name,
                    attributes,
                    children,
                } => {
                    pending.extend(children);
                    17 + name.namespace_uri.len() as u64
                        + name.local_name.len() as u64
                        + attributes
                            .iter()
                            .map(|(name, value)| {
                                24 + name.namespace_uri.len() as u64
                                    + name.local_name.len() as u64
                                    + value.len() as u64
                            })
                            .sum::<u64>()
                }
            });
        }
        total
    }
}
fn add_substitution(root: &mut Occurrence, step: SubstitutionStep) {
    let mut pending = vec![root];
    while let Some(item) = pending.pop() {
        item.trace.substitution_chain.push(step.clone());
        if let OccurrenceKind::Element { children, .. } = &mut item.kind {
            pending.extend(children);
        }
    }
}
fn valid_xml_chars(value: &str) -> bool {
    value.chars().all(|c| matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))
}

fn region(
    program: &LinkedProgram,
    at: squish_ir::LinkedRegionRef,
) -> Result<&squish_ir::Region, InstantiateError> {
    let unit = program
        .unit(at.unit_slot)
        .ok_or_else(|| InstantiateError::new("RUN023", "region unit is missing"))?;
    unit.regions()
        .get(at.region.0 as usize)
        .ok_or_else(|| InstantiateError::new("RUN024", "region is out of range"))
}
fn operation(program: &LinkedProgram, at: LinkedOpRef) -> Result<&Op, InstantiateError> {
    let unit = program
        .unit(at.unit_slot)
        .ok_or_else(|| InstantiateError::new("RUN025", "operation unit is missing"))?;
    unit.ops()
        .get(at.op.0 as usize)
        .map(|x| &x.op)
        .ok_or_else(|| InstantiateError::new("RUN026", "operation is out of range"))
}

fn origin(
    program: &LinkedProgram,
    slot: u32,
    kind: EntityKind,
    local: u32,
) -> Option<QualifiedOriginRef> {
    program.origin(slot, kind, local)
}
fn origin_for_op(program: &LinkedProgram, at: LinkedOpRef) -> Option<QualifiedOriginRef> {
    origin(program, at.unit_slot, EntityKind::Operation, at.op.0)
}
fn origin_for_region(
    program: &LinkedProgram,
    at: squish_ir::LinkedRegionRef,
) -> Option<QualifiedOriginRef> {
    origin(program, at.unit_slot, EntityKind::Region, at.region.0).or_else(|| {
        let r = region(program, at).ok()?;
        origin_for_op(
            program,
            LinkedOpRef {
                unit_slot: at.unit_slot,
                op: *r.ops.first()?,
            },
        )
    })
}
fn origin_for_definition(program: &LinkedProgram, at: DefAddr) -> Option<QualifiedOriginRef> {
    origin(
        program,
        at.unit_slot,
        EntityKind::Definition,
        at.local_def.0,
    )
}

fn file_binding(
    source: &squish_ir::SourceKey,
    kind: FileBinding,
) -> Result<String, InstantiateError> {
    let (uri, name) = match source {
        squish_ir::SourceKey::AdHoc { uri } => (
            uri.clone(),
            decode_uri_segment(uri.rsplit('/').next().unwrap_or(""))?,
        ),
        squish_ir::SourceKey::Project { package, path } => {
            let package = squish_source::PackageId::new(package.package_name.clone())
                .map_err(|e| InstantiateError::new("RUN030", e.to_string()))?;
            let logical = squish_source::LogicalPath::new(path.join("/"))
                .map_err(|e| InstantiateError::new("RUN031", e.to_string()))?;
            let identity = squish_source::SourceId::new(package, logical);
            (
                identity.uri().to_owned(),
                path.last().cloned().unwrap_or_default(),
            )
        }
    };
    Ok(match kind {
        FileBinding::Uri => uri,
        FileBinding::Name => name,
        FileBinding::Dir => uri
            .rsplit_once('/')
            .map_or(String::new(), |x| format!("{}/", x.0)),
    })
}

fn decode_uri_segment(value: &str) -> Result<String, InstantiateError> {
    let mut bytes = Vec::with_capacity(value.len());
    let source = value.as_bytes();
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            let pair = source.get(index + 1..index + 3).ok_or_else(|| {
                InstantiateError::new("RUN032", "source URI has an incomplete percent escape")
            })?;
            let text = std::str::from_utf8(pair).map_err(|_| {
                InstantiateError::new("RUN032", "source URI has a non-ASCII percent escape")
            })?;
            bytes.push(u8::from_str_radix(text, 16).map_err(|_| {
                InstantiateError::new("RUN032", "source URI has an invalid percent escape")
            })?);
            index += 3;
        } else {
            bytes.push(source[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| InstantiateError::new("RUN033", "source URI filename is not UTF-8"))
}

struct Flattened {
    document: LinkedDocumentIr,
    traces: Vec<TraceRef>,
    sequences: Vec<Vec<DocumentItemId>>,
}

fn flatten(
    roots: Vec<Occurrence>,
    image: &squish_ir::LinkedImage,
    sequence_count: usize,
) -> Result<Flattened, InstantiateError> {
    enum FlatTask {
        Emit(Occurrence, Option<SequenceValueId>),
        End {
            region: DocumentRegionId,
            trace: TraceRef,
            sequence: Option<SequenceValueId>,
        },
    }
    let mut items = Vec::new();
    let mut traces = Vec::new();
    let mut sequences = vec![Vec::new(); sequence_count];
    let mut regions = vec![DocumentRegion {
        id: DocumentRegionId(0),
        start: 0,
        end: 0,
    }];
    let mut tasks = Vec::new();
    for x in roots.into_iter().rev() {
        tasks.push(FlatTask::Emit(x, None));
    }
    while let Some(task) = tasks.pop() {
        match task {
            FlatTask::Emit(occ, inherited) => {
                let sequence = occ.sequence.or(inherited);
                match occ.kind {
                    OccurrenceKind::Text(value) => {
                        record_sequence(&mut sequences, sequence, items.len());
                        items.push(TempItem::Text(value));
                        traces.push(occ.trace);
                    }
                    OccurrenceKind::Comment(value) => {
                        record_sequence(&mut sequences, sequence, items.len());
                        items.push(TempItem::Comment(value));
                        traces.push(occ.trace);
                    }
                    OccurrenceKind::Pi(target, data) => {
                        record_sequence(&mut sequences, sequence, items.len());
                        items.push(TempItem::Pi(target, data));
                        traces.push(occ.trace);
                    }
                    OccurrenceKind::Element {
                        name,
                        attributes,
                        children,
                    } => {
                        let region = DocumentRegionId(regions.len() as u32);
                        regions.push(DocumentRegion {
                            id: region,
                            start: items.len() as u32 + 1,
                            end: 0,
                        });
                        record_sequence(&mut sequences, sequence, items.len());
                        items.push(TempItem::Start(name, attributes, region));
                        traces.push(occ.trace.clone());
                        tasks.push(FlatTask::End {
                            region,
                            trace: occ.trace,
                            sequence,
                        });
                        for child in children.into_iter().rev() {
                            tasks.push(FlatTask::Emit(child, sequence));
                        }
                    }
                }
            }
            FlatTask::End {
                region,
                trace,
                sequence,
            } => {
                regions[region.0 as usize].end = items.len() as u32;
                record_sequence(&mut sequences, sequence, items.len());
                items.push(TempItem::End);
                traces.push(trace);
            }
        }
    }
    regions[0].end = items.len() as u32;
    let (strings, qnames) = canonical_pools(&items);
    let items = items
        .into_iter()
        .map(|x| x.finish(&strings, &qnames))
        .collect();
    Ok(Flattened {
        document: LinkedDocumentIr {
            schema: image.schema,
            document_abi: squish_ir::AbiId(squish_ir::DOCUMENT_ABI.into()),
            root: DocumentRegionId(0),
            regions,
            items,
            strings,
            qnames,
            feature_bits: image.feature_bits,
        },
        traces,
        sequences,
    })
}

fn record_sequence(
    sequences: &mut [Vec<DocumentItemId>],
    sequence: Option<SequenceValueId>,
    item: usize,
) {
    if let Some(sequence) = sequence {
        sequences[sequence.0 as usize].push(DocumentItemId(item as u32));
    }
}

enum TempItem {
    Text(Text),
    Comment(Text),
    Pi(Text, Text),
    Start(
        squish_ir::ExpandedName,
        Vec<(squish_ir::ExpandedName, Text)>,
        DocumentRegionId,
    ),
    End,
}
impl TempItem {
    /// Borrows string candidates without allocating a per-occurrence temporary vector.
    fn strings(&self) -> impl Iterator<Item = &str> {
        let (first, second, attributes) = match self {
            Self::Text(text) | Self::Comment(text) => (Some(text), None, &[][..]),
            Self::Pi(target, data) => (Some(target), Some(data), &[][..]),
            Self::Start(_, attributes, _) => (None, None, attributes.as_slice()),
            Self::End => (None, None, &[][..]),
        };
        first
            .into_iter()
            .chain(second)
            .chain(attributes.iter().map(|attribute| &attribute.1))
            .map(Text::as_str)
    }

    /// Borrows element/attribute names; only canonical unique names become pool owners.
    fn qnames(&self) -> impl Iterator<Item = &squish_ir::ExpandedName> {
        let (name, attributes) = match self {
            Self::Start(name, attributes, _) => (Some(name), attributes.as_slice()),
            _ => (None, &[][..]),
        };
        name.into_iter()
            .chain(attributes.iter().map(|attribute| &attribute.0))
    }
    fn finish(self, strings: &[String], qnames: &[squish_ir::ExpandedName]) -> DocumentItem {
        match self {
            Self::Text(v) => DocumentItem::Text {
                value: StringId(
                    strings
                        .binary_search_by(|text| text.as_str().cmp(v.as_str()))
                        .unwrap() as u32,
                ),
            },
            Self::Comment(v) => DocumentItem::Comment {
                value: StringId(
                    strings
                        .binary_search_by(|text| text.as_str().cmp(v.as_str()))
                        .unwrap() as u32,
                ),
            },
            Self::Pi(target, data) => DocumentItem::ProcessingInstruction {
                target: StringId(
                    strings
                        .binary_search_by(|text| text.as_str().cmp(target.as_str()))
                        .unwrap() as u32,
                ),
                data: StringId(
                    strings
                        .binary_search_by(|text| text.as_str().cmp(data.as_str()))
                        .unwrap() as u32,
                ),
            },
            Self::Start(n, a, children) => DocumentItem::ElementStart {
                expanded_name: squish_ir::QNameId(qnames.binary_search(&n).unwrap() as u32),
                attributes: a
                    .into_iter()
                    .map(|(n, v)| squish_ir::Attribute {
                        name: squish_ir::QNameId(qnames.binary_search(&n).unwrap() as u32),
                        value: StringId(
                            strings
                                .binary_search_by(|text| text.as_str().cmp(v.as_str()))
                                .unwrap() as u32,
                        ),
                    })
                    .collect(),
                children,
            },
            Self::End => DocumentItem::ElementEnd,
        }
    }
}

/// Canonicalizes borrowed candidates before cloning only the unique output pool payloads.
///
/// Temporary items retain their owned occurrence strings until IDs are resolved. Reference
/// sorting/deduplication compares values, not addresses, preserving the previous wire ordering.
/// No fragment or occurrence is discarded: only the document's interned pools are deduplicated.
fn canonical_pools(items: &[TempItem]) -> (Vec<String>, Vec<squish_ir::ExpandedName>) {
    let mut strings: Vec<_> = items.iter().flat_map(TempItem::strings).collect();
    strings.sort();
    strings.dedup();
    let strings = strings.into_iter().map(str::to_owned).collect();
    let mut qnames: Vec<_> = items.iter().flat_map(TempItem::qnames).collect();
    qnames.sort();
    qnames.dedup();
    let qnames = qnames.into_iter().cloned().collect();
    (strings, qnames)
}

fn validate_document_shape(
    document: &LinkedDocumentIr,
    root_kind: squish_ir::UnitKind,
) -> Result<(), InstantiateError> {
    // Archive units produce member directives, not XML documents. Their neutral event
    // tape may be an empty/trivia fragment; the manager separately enforces archive
    // content policy. Prompt entries retain their strict single-document contract.
    if matches!(
        root_kind,
        squish_ir::UnitKind::Pack | squish_ir::UnitKind::Sopack
    ) {
        return Ok(());
    }
    let mut depth = 0usize;
    let mut roots = 0usize;
    for item in &document.items {
        match item {
            DocumentItem::ElementStart { .. } => {
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
            }
            DocumentItem::ElementEnd => depth -= 1,
            DocumentItem::Text { value }
                if depth == 0
                    && !document.strings[value.0 as usize]
                        .chars()
                        .all(char::is_whitespace) =>
            {
                return Err(InstantiateError::new(
                    "RUN028",
                    "non-whitespace text occurs outside the document element",
                ));
            }
            _ => {}
        }
    }
    if roots != 1 {
        return Err(InstantiateError::new(
            "RUN029",
            "result must contain exactly one document element",
        ));
    }
    Ok(())
}

fn canonicalize_document(_: &mut LinkedDocumentIr) {}
fn canonicalize_scalars(
    values: Vec<Text>,
    frames: &mut [FrameRecord],
    origins: &mut [OriginNode],
) -> Vec<String> {
    // Group visible values, release duplicate handles, then materialize each unique public
    // String once. Full unique buffers can move; a partial view copies only its visible bytes.
    let mut indexed: Vec<_> = values.into_iter().enumerate().collect();
    indexed.sort_unstable_by(|(_, left), (_, right)| left.as_str().cmp(right.as_str()));
    let mut ids = vec![ScalarValueId(0); indexed.len()];
    let mut strings = Vec::with_capacity(indexed.len());
    let mut indexed = indexed.into_iter().peekable();
    while let Some((old_id, text)) = indexed.next() {
        let id = ScalarValueId(strings.len() as u32);
        ids[old_id] = id;
        while indexed
            .peek()
            .is_some_and(|(_, next)| next.as_str() == text.as_str())
        {
            let (duplicate_id, _) = indexed.next().expect("peeked scalar value");
            ids[duplicate_id] = id;
        }
        strings.push(text.into_owned());
    }
    let remap = |id: &mut ScalarValueId| {
        *id = ids[id.0 as usize];
    };
    for frame in frames {
        for (_, id) in &mut frame.args {
            remap(id);
        }
    }
    for origin in origins {
        if let OriginNode::ExternalArgument { value, .. } = origin {
            remap(value);
        }
    }
    strings
}

#[cfg(test)]
mod archive_fragment_tests {
    use super::*;
    use squish_ir::{AbiId, DocumentRegion, FeatureBits, UnitKind, Version};

    fn empty_fragment() -> LinkedDocumentIr {
        LinkedDocumentIr {
            schema: Version { major: 1, minor: 0 },
            document_abi: AbiId(squish_ir::DOCUMENT_ABI.into()),
            root: DocumentRegionId(0),
            regions: vec![DocumentRegion {
                id: DocumentRegionId(0),
                start: 0,
                end: 0,
            }],
            items: Vec::new(),
            strings: Vec::new(),
            qnames: Vec::new(),
            feature_bits: FeatureBits(0),
        }
    }
    #[test]
    fn archive_empty_fragments_do_not_weaken_prompt_entry_shape() {
        let document = empty_fragment();
        document.validate().unwrap();
        assert!(validate_document_shape(&document, UnitKind::Pack).is_ok());
        assert!(validate_document_shape(&document, UnitKind::Sopack).is_ok());
        assert_eq!(
            validate_document_shape(&document, UnitKind::Entry)
                .unwrap_err()
                .code,
            "RUN029"
        );
    }
    #[test]
    fn archive_trivia_fragments_remain_valid_neutral_ir() {
        let mut document = empty_fragment();
        document.strings.push(" \n".into());
        document
            .items
            .push(DocumentItem::Text { value: StringId(0) });
        document.regions[0].end = 1;
        document.validate().unwrap();
        assert!(validate_document_shape(&document, UnitKind::Pack).is_ok());
        assert_eq!(
            validate_document_shape(&document, UnitKind::Entry)
                .unwrap_err()
                .code,
            "RUN029"
        );
        document.strings[0] = "not a document".into();
        assert_eq!(
            validate_document_shape(&document, UnitKind::Entry)
                .unwrap_err()
                .code,
            "RUN028"
        );
    }
}

#[cfg(test)]
mod scalar_index_tests {
    use super::*;

    #[test]
    fn repeated_large_single_value_never_hashes_or_duplicates_an_index_key() {
        let text: Text = "猫".repeat(65536 / 3).into();
        let mut values = Vec::new();
        let mut index = ScalarIndex::default();
        for _ in 0..1024 {
            assert_eq!(index.intern(&mut values, &text), 0);
        }
        assert_eq!(values, [text]);
        assert!(index.large.is_none());
    }

    #[test]
    fn promotion_preserves_duplicate_prefix_first_ids_and_indexes_new_values() {
        // Entry setup may retain duplicates. Promotion must not select the last equal ID.
        let mut values: Vec<Text> = vec![String::new().into(), "same".into(), "same".into()];
        values.extend((0..LINEAR_SCALAR_LIMIT).map(|id| Text::from(format!("prefix{id}"))));
        let mut index = ScalarIndex::default();
        assert_eq!(index.intern(&mut values, &Text::from("same")), 1);
        assert_eq!(index.intern(&mut values, &Text::from("")), 0);
        assert!(index.large.is_some());
        for id in 0..2048 {
            let text: Text = format!("different-{id:08}").into();
            let first = values.len() as u32;
            assert_eq!(index.intern(&mut values, &text), first);
            assert_eq!(index.intern(&mut values, &text), first);
        }
        for (id, text) in values.iter().enumerate() {
            let expected = values.iter().position(|old| old == text).unwrap() as u32;
            assert_eq!(index.find(&values, text), Some(expected), "arena slot {id}");
        }
    }

    #[test]
    fn bounded_prefix_promotes_only_after_the_crossover_and_keeps_empty_unicode_ids() {
        let mut values = Vec::new();
        let mut index = ScalarIndex::default();
        let text: Vec<_> = (0..=LINEAR_SCALAR_LIMIT)
            .map(|id| {
                Text::from(if id == 0 {
                    String::new()
                } else {
                    format!("猫{id}")
                })
            })
            .collect();
        for (id, value) in text.iter().enumerate() {
            assert_eq!(index.intern(&mut values, value), id as u32);
            assert!(index.large.is_none());
        }
        assert_eq!(index.intern(&mut values, &text[1]), 1);
        assert!(index.large.is_some());
        assert_eq!(
            index.intern(&mut values, &Text::from("after")),
            text.len() as u32
        );
        assert_eq!(
            index.intern(&mut values, &Text::from("after")),
            text.len() as u32
        );
    }
}

#[cfg(test)]
mod canonical_pool_tests {
    use super::*;
    use squish_ir::{Attribute, ExpandedName, QNameId};

    /// Builds names with intentionally non-sorted input positions and repeated identities.
    fn name(local_name: &str) -> ExpandedName {
        ExpandedName {
            namespace_uri: "urn:test".into(),
            local_name: local_name.into(),
        }
    }

    #[test]
    fn borrowed_candidates_keep_unique_unicode_pi_attribute_and_comment_pools() {
        let long = "猫<&>".repeat(16384);
        let mut items: Vec<_> = (0..32)
            .map(|_| TempItem::Text(long.clone().into()))
            .collect();
        items.extend([
            TempItem::Comment(long.clone().into()),
            TempItem::Pi("target".into(), String::new().into()),
            TempItem::Start(
                name("z"),
                vec![
                    (name("a"), long.clone().into()),
                    (name("z"), "alpha".into()),
                ],
                DocumentRegionId(1),
            ),
            TempItem::Text("alpha".into()),
            TempItem::End,
        ]);
        let (strings, qnames) = canonical_pools(&items);
        assert_eq!(
            strings,
            [String::new(), "alpha".into(), "target".into(), long]
        );
        assert_eq!(qnames, [name("a"), name("z")]);
        let finished: Vec<_> = items
            .into_iter()
            .map(|item| item.finish(&strings, &qnames))
            .collect();
        assert!(
            finished[..32]
                .iter()
                .all(|item| *item == DocumentItem::Text { value: StringId(3) })
        );
        assert_eq!(finished[32], DocumentItem::Comment { value: StringId(3) });
        assert_eq!(
            finished[33],
            DocumentItem::ProcessingInstruction {
                target: StringId(2),
                data: StringId(0)
            }
        );
        assert_eq!(
            finished[34],
            DocumentItem::ElementStart {
                expanded_name: QNameId(1),
                attributes: vec![
                    Attribute {
                        name: QNameId(0),
                        value: StringId(3)
                    },
                    Attribute {
                        name: QNameId(1),
                        value: StringId(1)
                    }
                ],
                children: DocumentRegionId(1),
            }
        );
        assert_eq!(finished[35], DocumentItem::Text { value: StringId(1) });
        assert_eq!(finished[36], DocumentItem::ElementEnd);
    }

    #[test]
    fn empty_and_all_distinct_candidates_remain_exactly_sorted_without_dropping_items() {
        assert_eq!(canonical_pools(&[]), (Vec::new(), Vec::new()));
        let items = [
            TempItem::Text("猫".into()),
            TempItem::Text("z".into()),
            TempItem::Text(String::new().into()),
        ];
        let (strings, qnames) = canonical_pools(&items);
        assert_eq!(strings, ["", "z", "猫"]);
        assert!(qnames.is_empty());
        let finished: Vec<_> = items
            .into_iter()
            .map(|item| item.finish(&strings, &qnames))
            .collect();
        assert_eq!(
            finished,
            [
                DocumentItem::Text { value: StringId(2) },
                DocumentItem::Text { value: StringId(1) },
                DocumentItem::Text { value: StringId(0) },
            ]
        );
    }
}

#[cfg(test)]
mod shared_text_tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    /// Uses the same str hashing contract as borrowed scalar-index lookup.
    fn hash(value: impl Hash) -> u64 {
        let mut state = DefaultHasher::new();
        value.hash(&mut state);
        state.finish()
    }

    #[test]
    fn utf8_views_hash_and_compare_visible_bytes_not_backing_or_offset() {
        let backing = Arc::new("prefix猫suffix".to_owned());
        let view = Text::view(Arc::clone(&backing), 6..9).unwrap();
        let owned = Text::from("猫");
        assert_eq!(view, owned);
        assert_eq!(hash(&view), hash("猫"));
        assert_eq!(hash(&view), hash(&owned));
        assert_eq!(<Text as Borrow<str>>::borrow(&view), "猫");
        assert!(Text::view(Arc::clone(&backing), 7..9).is_none());
        assert!(Text::view(Arc::clone(&backing), 0..usize::MAX).is_none());
        assert!(view.slice(1..2).is_none());
        let surrounding_invalid = Arc::new("\0猫\0".to_owned());
        let visible = Text::view(Arc::clone(&surrounding_invalid), 1..4).unwrap();
        assert!(valid_xml_chars(visible.as_str()));
        assert!(!valid_xml_chars(surrounding_invalid.as_str()));
        let mut values: Vec<Text> = (0..9).map(|id| format!("item{id}").into()).collect();
        let mut index = ScalarIndex::default();
        assert_eq!(index.intern(&mut values, &view), 9);
        assert_eq!(index.intern(&mut values, &owned), 9);
        assert_eq!(index.large.as_ref().unwrap().get("猫"), Some(&9));
        assert_eq!(index.large.as_ref().unwrap().get(backing.as_str()), None);
        assert!(Arc::ptr_eq(values[9].owned_backing().unwrap(), &backing));
        let strings = canonicalize_scalars(vec![view, owned, Text::from("")], &mut [], &mut []);
        assert_eq!(strings, ["", "猫"]);
    }

    #[test]
    fn scalar_concatenation_retains_single_views_and_copies_only_visible_multi_piece_bytes() {
        let backing = Arc::new(format!("{}猫tail", "p".repeat(1024 * 1024)));
        let view = Text::view(Arc::clone(&backing), 1024 * 1024..1024 * 1024 + 3).unwrap();
        let mut single = ScalarText::default();
        single.append(Text::from(""));
        single.append(view.clone());
        single.append(Text::from(""));
        let single = single.finish();
        assert!(Arc::ptr_eq(single.owned_backing().unwrap(), &backing));
        assert_eq!(single.as_str(), "猫");
        let mut joined = ScalarText::default();
        joined.append(view.clone());
        joined.append(Text::from(""));
        joined.append(Text::from("猫"));
        let joined = joined.finish();
        assert_eq!(joined.as_str(), "猫猫");
        assert_eq!(joined.backing.as_str().len(), 6);
        assert!(!Arc::ptr_eq(joined.owned_backing().unwrap(), &backing));
        assert_eq!(view.into_owned(), "猫");
        let owned = Text::from("transfer this buffer");
        let pointer = owned.backing.as_str().as_ptr();
        assert_eq!(owned.into_owned().as_ptr(), pointer);
        assert_eq!(ScalarText::default().finish().as_str(), "");
    }

    #[test]
    fn captures_slice_relative_visible_input_and_clones_keep_independent_provenance() {
        let backing = Arc::new(format!("{}a猫zsuffix", "p".repeat(1024 * 1024)));
        let input = Text::view(Arc::clone(&backing), 1024 * 1024..1024 * 1024 + 5).unwrap();
        let regex = Regex::new("(?P<value>猫)").unwrap();
        let capture = regex
            .captures(input.as_str())
            .unwrap()
            .name("value")
            .unwrap();
        let text = input.slice(capture.start()..capture.end()).unwrap();
        assert_eq!(text.as_str(), "猫");
        assert!(Arc::ptr_eq(text.owned_backing().unwrap(), &backing));
        let origin = QualifiedOriginRef {
            object: squish_ir::ObjectDigest::of(b"shared-text-test"),
            local: squish_ir::OriginId(0),
        };
        let occurrence = Occurrence {
            kind: OccurrenceKind::Text(text),
            trace: TraceRef {
                producer_op: LinkedOpRef {
                    unit_slot: 0,
                    op: squish_ir::OpId(0),
                },
                frame: FrameId(0),
                definition_origin: origin.clone(),
                call_origin: None,
                substitution_chain: Vec::new(),
            },
            sequence: None,
        };
        let mut copy = occurrence.clone();
        add_substitution(
            &mut copy,
            SubstitutionStep {
                kind: SubstitutionKind::SlotFill,
                origin,
            },
        );
        assert!(occurrence.trace.substitution_chain.is_empty());
        assert_eq!(copy.trace.substitution_chain.len(), 1);
        let (OccurrenceKind::Text(first), OccurrenceKind::Text(second)) =
            (&occurrence.kind, &copy.kind)
        else {
            unreachable!()
        };
        assert!(Arc::ptr_eq(
            first.owned_backing().unwrap(),
            second.owned_backing().unwrap()
        ));
        assert_eq!(occurrence.cost(), copy.cost());
        let strings = canonical_pools(&[
            TempItem::Text(first.clone()),
            TempItem::Text(second.clone()),
        ])
        .0;
        assert_eq!(strings, ["猫"]);
    }
}

#[cfg(test)]
mod unit_text_retention_tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    #[test]
    fn repeated_tiny_literal_captures_share_the_already_program_owned_unit_without_string_backings()
    {
        let program = crate::tests::linked_large_literal_captures(16);
        let slot = program
            .image()
            .units
            .iter()
            .position(|unit| unit.kind == squish_ir::UnitKind::Module)
            .unwrap() as u32;
        let unit = program.unit_arc(slot).unwrap();
        let id = StringId(
            unit.header()
                .semantic_strings
                .iter()
                .position(|text| text.len() > 1024 * 1024)
                .unwrap() as u32,
        );
        let len = unit.header().semantic_strings[id.0 as usize].len();
        let input = Text::unit_view(Arc::clone(&unit), id, 0..len).unwrap();
        let regex = Regex::new("(?P<value>猫)").unwrap();
        let capture = regex
            .captures(input.as_str())
            .unwrap()
            .name("value")
            .unwrap();
        let views: Vec<_> = (0..1024)
            .map(|_| input.slice(capture.start()..capture.end()).unwrap())
            .collect();
        for view in &views {
            assert_eq!(view.as_str(), "猫");
            assert!(view.owned_backing().is_none());
            let TextBacking::Unit {
                unit: owner,
                string,
            } = &view.backing
            else {
                unreachable!()
            };
            assert!(Arc::ptr_eq(owner, &unit));
            assert_eq!(*string, id);
            assert_eq!(view, &Text::from("猫"));
            let mut actual = DefaultHasher::new();
            let mut expected = DefaultHasher::new();
            view.hash(&mut actual);
            "猫".hash(&mut expected);
            assert_eq!(actual.finish(), expected.finish());
            assert_eq!(<Text as Borrow<str>>::borrow(view), "猫");
            let debug = format!("{view:?}");
            assert!(debug.contains("猫"));
            assert!(!debug.contains("pppp"));
            assert!(!debug.contains("semantic_strings"));
        }
        assert_eq!(
            canonical_pools(
                &views
                    .iter()
                    .cloned()
                    .map(TempItem::Text)
                    .collect::<Vec<_>>()
            )
            .0,
            ["猫"]
        );
        assert!(Text::unit_view(Arc::clone(&unit), StringId(u32::MAX), 0..0).is_none());
        assert!(Text::unit_view(Arc::clone(&unit), id, 0..len + 1).is_none());
        assert!(
            Text::unit_view(Arc::clone(&unit), id, capture.start() + 1..capture.end()).is_none()
        );
    }
}
