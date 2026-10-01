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
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
    rc::Rc,
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

#[derive(Clone, Debug)]
struct Value {
    text: String,
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
    Text(String),
    Comment(String),
    Pi(String, String),
    Element {
        name: squish_ir::ExpandedName,
        attributes: Vec<(squish_ir::ExpandedName, String)>,
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
        attrs: Vec<(squish_ir::ExpandedName, String)>,
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

struct Machine<'a> {
    program: &'a LinkedProgram,
    budgets: Budgets,
    tasks: Vec<Task<'a>>,
    buffers: Vec<Vec<Occurrence>>,
    buffer_bytes: Vec<u64>,
    frames: Vec<FrameRecord>,
    frame_origins: Vec<QualifiedOriginRef>,
    scalar_values: Vec<String>,
    /// Session-only value lookup preserving first-insertion arena IDs; never serialized.
    scalar_index: HashMap<String, u32>,
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
        let mut scalar_index = HashMap::new();
        let values: BTreeMap<_, _> = args
            .into_iter()
            .map(|(name, text)| {
                scalar_index
                    .entry(text.clone())
                    .or_insert(scalar_values.len() as u32);
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
        let frame_args: Vec<(LocalName, ScalarValueId)> = values
            .iter()
            .map(|(n, v)| (n.clone(), ScalarValueId(scalar_index[&v.text])))
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
                                            literal.text[segment.bytes.clone()].to_owned(),
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
                    let mut text = String::new();
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
                        text.push_str(&value);
                        substitutions.extend(occurrence.trace.substitution_chain);
                    }
                    substitutions.push(SubstitutionStep {
                        kind: SubstitutionKind::ScalarBody,
                        origin: self.op_origin(call.op_ref)?,
                    });
                    call.values.insert(
                        call.args[index].name.clone(),
                        Value {
                            text,
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
        canonicalize_scalars(&mut self.scalar_values, &mut self.frames, &mut self.origins);
        let trace = ExpansionTrace {
            frames: std::mem::take(&mut self.frames),
            scalar_values: std::mem::take(&mut self.scalar_values),
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
                OccurrenceKind::Text(self.string(at.unit_slot, *value)?),
            )?,
            Op::EmitComment { value } => self.emit_simple(
                at,
                &env,
                output,
                OccurrenceKind::Comment(self.string(at.unit_slot, *value)?),
            )?,
            Op::EmitPi { target, data } => self.emit_simple(
                at,
                &env,
                output,
                OccurrenceKind::Pi(
                    self.string(at.unit_slot, *target)?,
                    self.string(at.unit_slot, *data)?,
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
                            self.string(at.unit_slot, a.value)?,
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
                        text: self.string(at.unit_slot, *id)?,
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
                                    text: input.text[value.clone()].into(),
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
                text: self.string(slot, *id)?,
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
                    text: file_binding(&self.source(slot)?, *kind)?,
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
    fn intern_scalar(&mut self, value: &str) -> u32 {
        if let Some(id) = self.scalar_index.get(value) {
            return *id;
        }
        let id = self.scalar_values.len() as u32;
        self.scalar_values.push(value.into());
        self.scalar_index.insert(value.into(), id);
        id
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
    let mut strings: Vec<_> = items.iter().flat_map(TempItem::strings).collect();
    strings.sort();
    strings.dedup();
    let mut qnames: Vec<_> = items.iter().flat_map(TempItem::qnames).collect();
    qnames.sort();
    qnames.dedup();
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
    Text(String),
    Comment(String),
    Pi(String, String),
    Start(
        squish_ir::ExpandedName,
        Vec<(squish_ir::ExpandedName, String)>,
        DocumentRegionId,
    ),
    End,
}
impl TempItem {
    fn strings(&self) -> Vec<String> {
        match self {
            Self::Text(x) | Self::Comment(x) => vec![x.clone()],
            Self::Pi(a, b) => vec![a.clone(), b.clone()],
            Self::Start(_, a, _) => a.iter().map(|x| x.1.clone()).collect(),
            Self::End => vec![],
        }
    }
    fn qnames(&self) -> Vec<squish_ir::ExpandedName> {
        match self {
            Self::Start(n, a, _) => std::iter::once(n.clone())
                .chain(a.iter().map(|x| x.0.clone()))
                .collect(),
            _ => vec![],
        }
    }
    fn finish(self, strings: &[String], qnames: &[squish_ir::ExpandedName]) -> DocumentItem {
        match self {
            Self::Text(v) => DocumentItem::Text {
                value: StringId(strings.binary_search(&v).unwrap() as u32),
            },
            Self::Comment(v) => DocumentItem::Comment {
                value: StringId(strings.binary_search(&v).unwrap() as u32),
            },
            Self::Pi(target, data) => DocumentItem::ProcessingInstruction {
                target: StringId(strings.binary_search(&target).unwrap() as u32),
                data: StringId(strings.binary_search(&data).unwrap() as u32),
            },
            Self::Start(n, a, children) => DocumentItem::ElementStart {
                expanded_name: squish_ir::QNameId(qnames.binary_search(&n).unwrap() as u32),
                attributes: a
                    .into_iter()
                    .map(|(n, v)| squish_ir::Attribute {
                        name: squish_ir::QNameId(qnames.binary_search(&n).unwrap() as u32),
                        value: StringId(strings.binary_search(&v).unwrap() as u32),
                    })
                    .collect(),
                children,
            },
            Self::End => DocumentItem::ElementEnd,
        }
    }
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
    values: &mut Vec<String>,
    frames: &mut [FrameRecord],
    origins: &mut [OriginNode],
) {
    // Sort indices, then move each owned string once. Retain an explicit old-ID mapping
    // rather than cloning all text or performing a binary search for every frame reference.
    let mut order: Vec<_> = (0..values.len()).collect();
    order.sort_unstable_by(|a, b| values[*a].cmp(&values[*b]));
    let mut old = std::mem::take(values);
    values.reserve(old.len());
    let mut ids = vec![ScalarValueId(0); old.len()];
    for index in order {
        let value = std::mem::take(&mut old[index]);
        if values.last() != Some(&value) {
            values.push(value);
        }
        ids[index] = ScalarValueId((values.len() - 1) as u32);
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
