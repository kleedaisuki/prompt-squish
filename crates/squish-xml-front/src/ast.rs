//! 前端私有的损耗性 AST。 / Private, disposable frontend AST.

use squish_ir::{DecodedSyntax, ExpandedName, ImportSpec};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Loc {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
pub(crate) struct Node {
    pub loc: Loc,
    pub lexical: Option<String>,
    pub kind: Kind,
    pub decoded: Vec<RawDecodedValue>,
}

#[derive(Clone, Debug)]
pub(crate) struct RawDecodedValue {
    pub field: String,
    pub segments: Vec<RawDecodedSegment>,
}

#[derive(Clone, Debug)]
pub(crate) struct RawDecodedSegment {
    pub value_start: usize,
    pub value_end: usize,
    pub source_start: usize,
    pub source_end: usize,
    pub syntax: DecodedSyntax,
}
impl Node {
    /// 在保留非递归 Drop 契约的同时取出 payload。 / Takes payload while preserving flat drop.
    pub(crate) fn take(mut self) -> (Loc, Option<String>, Kind, Vec<RawDecodedValue>) {
        let loc = self.loc;
        let lexical = self.lexical.take();
        let kind = std::mem::replace(&mut self.kind, Kind::Text(String::new()));
        let decoded = std::mem::take(&mut self.decoded);
        (loc, lexical, kind, decoded)
    }
}

#[derive(Debug)]
pub(crate) enum Kind {
    Text(String),
    Comment(String),
    Pi {
        target: String,
        data: String,
    },
    Element {
        name: ExpandedName,
        attrs: Vec<(ExpandedName, String)>,
        children: Vec<Node>,
    },
    Insert(String),
    If {
        input: Value,
        pattern: String,
        captures: Vec<String>,
        body: Vec<Node>,
    },
    Slot {
        name: String,
    },
    Invoke {
        target: ExpandedName,
        args: Vec<Argument>,
        fills: Vec<Fill>,
    },
}

#[derive(Debug)]
pub(crate) enum Value {
    Literal(String),
    Get(String),
    Body(Vec<Node>),
}
#[derive(Debug)]
pub(crate) struct Argument {
    pub name: String,
    pub value: Value,
}
#[derive(Debug)]
pub(crate) struct Fill {
    pub name: String,
    pub body: Vec<Node>,
}
#[derive(Debug)]
pub(crate) struct Definition {
    pub loc: Loc,
    pub lexical: String,
    pub symbol: ExpandedName,
    pub params: Vec<String>,
    pub slots: BTreeMap<String, bool>,
    pub body: Vec<Node>,
    pub decoded: Vec<RawDecodedValue>,
}
#[derive(Debug)]
pub(crate) struct Import {
    pub loc: Loc,
    pub spec: ImportSpec,
    pub decoded: RawDecodedValue,
}
#[derive(Debug)]
pub(crate) enum Root {
    Entry {
        params: Vec<String>,
        param_origins: Vec<(Loc, RawDecodedValue)>,
        body: Vec<Node>,
    },
    Module {
        definitions: Vec<Definition>,
    },
}
#[derive(Debug)]
pub(crate) struct Unit {
    pub root: Root,
    pub imports: Vec<Import>,
}

impl Drop for Node {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        detach(&mut self.kind, &mut pending);
        while let Some(mut node) = pending.pop() {
            detach(&mut node.kind, &mut pending)
        }
    }
}
fn detach(kind: &mut Kind, pending: &mut Vec<Node>) {
    match kind {
        Kind::Element { children, .. } | Kind::If { body: children, .. } => {
            pending.append(children)
        }
        Kind::Invoke { args, fills, .. } => {
            for arg in args {
                if let Value::Body(v) = &mut arg.value {
                    pending.append(v)
                }
            }
            for fill in fills {
                pending.append(&mut fill.body)
            }
        }
        _ => {}
    }
}
