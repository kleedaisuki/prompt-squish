//! Iterative, caller-evaluated expansion and provenance-preserving lowering.
//! 迭代展开：参数与填充在调用方求值，降级时保留可追溯信息。
use super::*;
use std::{collections::BTreeMap, rc::Rc};

/// Flat output event; ownership never forms recursive trees. / 扁平输出事件，避免递归树所有权。
#[derive(Clone)]
struct Token<'a> {
    /// Serialized XML and optional decoded character data. / XML 序列化与可选解码文本。
    xml: Rc<str>,
    text: Option<Rc<str>>,
    /// Original generation site, retained through slot substitution. / 生成位置，slot 替换不修改。
    loc: &'a Loc,
    frame: usize,
    kind: &'static str,
}
/// A live lexical scope; captures do not cross calls. / 活跃词法作用域，捕获不跨调用。
#[derive(Clone)]
struct Env {
    frame: usize,
    captures: Rc<BTreeMap<String, String>>,
}
/// Invocation provenance and immutable inputs. / 调用来源及不可变输入。
struct Frame<'a> {
    def: usize,
    parent: Option<usize>,
    call: &'a Loc,
    depth: usize,
    args: BTreeMap<String, String>,
    slots: BTreeMap<String, Rc<Vec<Token<'a>>>>,
}
/// A bounded temporary or final output sequence. / 有界临时或最终输出序列。
#[derive(Default)]
struct Buffer<'a> {
    tokens: Vec<Token<'a>>,
    bytes: usize,
}
/// Suspended call construction, evaluated strictly in the caller. / 暂停的调用构造，严格在调用方求值。
struct Call<'a> {
    target: usize,
    node: &'a Node,
    env: Env,
    out: usize,
    args: &'a [Argument],
    fills: &'a [Fill],
    values: BTreeMap<String, String>,
    slots: BTreeMap<String, Rc<Vec<Token<'a>>>>,
}
/// Explicit continuations replace native recursion. / 显式续体代替本机递归。
enum Task<'a> {
    Node(&'a Node, Env, usize),
    Close(&'a Node, Env, usize, &'a str),
    Arg(Box<Call<'a>>, usize),
    ArgDone(Box<Call<'a>>, usize, usize),
    Fill(Box<Call<'a>>, usize),
    FillDone(Box<Call<'a>>, usize, usize),
}
/// Per-invocation state; definitions remain borrowed and frozen. / 单次执行状态，定义保持借用与冻结。
struct Machine<'a> {
    program: &'a Program,
    options: &'a CompileOptions,
    frames: Vec<Frame<'a>>,
    tasks: Vec<Task<'a>>,
    buffers: BTreeMap<usize, Buffer<'a>>,
    next_buffer: usize,
}
/// Escape character data without changing its Unicode value. / 转义字符数据而不改变 Unicode 值。
fn escape(value: &str, attr: bool) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\r' => out.push_str("&#13;"),
            '\n' if attr => out.push_str("&#10;"),
            '\t' if attr => out.push_str("&#9;"),
            _ => out.push(ch),
        }
    }
    out
}
impl<'a> Machine<'a> {
    /// Report a complete root-to-leaf frame chain. / 报告从根至叶的完整调用链。
    fn fail(&self, loc: &Loc, frame: usize, message: impl AsRef<str>) -> CompileError {
        let mut chain = Vec::new();
        let mut current = Some(frame);
        while let Some(id) = current {
            let f = &self.frames[id];
            let def = &self.program.defs[f.def];
            chain.push(format!(
                "\n  frame #{id}: call {}:{} [{}..{}], definition #{} {}:{} [{}..{}]",
                file_uri(&f.call.path).unwrap_or_else(|_| f.call.path.display().to_string()),
                f.call.line,
                f.call.start,
                f.call.end,
                f.def,
                file_uri(&def.loc.path).unwrap_or_else(|_| def.loc.path.display().to_string()),
                def.loc.line,
                def.loc.start,
                def.loc.end
            ));
            current = f.parent;
        }
        chain.reverse();
        CompileError {
            path: loc.path.clone(),
            line: loc.line,
            message: format!(
                "{} at {} [{}..{}]{}",
                message.as_ref(),
                file_uri(&loc.path).unwrap_or_else(|_| loc.path.display().to_string()),
                loc.start,
                loc.end,
                chain.concat()
            ),
        }
    }
    /// Allocate an independently guarded evaluation buffer. / 分配独立受限的求值缓冲。
    fn buffer(&mut self) -> usize {
        let id = self.next_buffer;
        self.next_buffer += 1;
        self.buffers.insert(id, Buffer::default());
        id
    }
    /// Schedule in source order using a LIFO work stack. / 使用后进先出工作栈保持源码顺序。
    fn schedule(&mut self, nodes: &'a [Node], env: &Env, out: usize) {
        self.tasks
            .extend(nodes.iter().rev().map(|n| Task::Node(n, env.clone(), out)));
    }
    /// Account before appending, so no successful result can exceed its budget. / 追加前计费，成功结果不越预算。
    fn emit(&mut self, out: usize, token: Token<'a>) -> Result<(), CompileError> {
        let loc = token.loc;
        let frame = token.frame;
        self.emit_at(out, token, loc, frame)
    }
    /// Separate execution diagnostics from immutable node provenance. / 将执行诊断与不可变节点来源分离。
    fn emit_at(
        &mut self,
        out: usize,
        token: Token<'a>,
        loc: &Loc,
        frame: usize,
    ) -> Result<(), CompileError> {
        let bytes = self.buffers[&out].bytes.checked_add(token.xml.len());
        if bytes.is_none_or(|b| b > self.options.max_output_bytes) {
            return Err(self.fail(loc, frame, "Expansion: max-output-bytes exceeded"));
        }
        let buffer = self.buffers.get_mut(&out).unwrap();
        buffer.bytes = bytes.unwrap();
        buffer.tokens.push(token);
        Ok(())
    }
    /// Resolve only the three language-defined scalar namespaces. / 仅解析语言规定的三种标量命名空间。
    fn get(&self, key: &str, env: &Env, loc: &Loc) -> Result<String, CompileError> {
        let f = &self.frames[env.frame];
        let path = &self.program.defs[f.def].loc.path;
        let value = match key.split_once('.') {
            Some(("arg", name)) => f.args.get(name).cloned(),
            Some(("match", name)) => env.captures.get(name).cloned(),
            Some(("file", "uri")) => {
                Some(file_uri(path).map_err(|e| self.fail(loc, env.frame, e))?)
            }
            Some(("file", "dir")) => {
                Some(directory_uri(path).map_err(|e| self.fail(loc, env.frame, e))?)
            }
            Some(("file", "name")) => Some(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
            _ => None,
        };
        value.ok_or_else(|| self.fail(loc, env.frame, format!("Scalar: undefined binding '{key}'")))
    }
    /// Evaluate an immediate scalar; body values use explicit continuations. / 求值即时标量，主体值使用显式续体。
    fn value(&self, value: &Value, env: &Env, loc: &Loc) -> Result<String, CompileError> {
        match value {
            Value::Literal(value) => Ok(value.clone()),
            Value::Get(key) => self.get(key, env, loc),
            Value::Body(_) => unreachable!("body values require a continuation"),
        }
    }
    /// Validate signatures after caller-side evaluation and before entering the callee. / 调用方求值后、进入被调方前验证签名。
    fn enter(&mut self, call: Call<'a>) -> Result<(), CompileError> {
        let def = &self.program.defs[call.target];
        let contract = |message: String| {
            self.fail(
                &call.node.loc,
                call.env.frame,
                format!(
                    "Signature: {message}; definition {}:{} [{}..{}]",
                    def.loc.path.display(),
                    def.loc.line,
                    def.loc.start,
                    def.loc.end
                ),
            )
        };
        for param in &def.params {
            if !call.values.contains_key(param) {
                return Err(contract(format!("missing argument '{param}'")));
            }
        }
        for name in call.values.keys() {
            if !def.params.contains(name) {
                return Err(contract(format!("unknown argument '{name}'")));
            }
        }
        for (name, required) in &def.slots {
            if *required && !call.slots.contains_key(name) {
                return Err(contract(format!("missing required fill '{name}'")));
            }
        }
        for name in call.slots.keys() {
            if !def.slots.contains_key(name) {
                return Err(contract(format!("unknown fill '{name}'")));
            }
        }
        let depth = self.frames[call.env.frame].depth + 1;
        let frame = self.frames.len();
        self.frames.push(Frame {
            def: call.target,
            parent: Some(call.env.frame),
            call: &call.node.loc,
            depth,
            args: call.values,
            slots: call.slots,
        });
        if depth > self.options.max_depth {
            return Err(self.fail(&call.node.loc, frame, "Expansion: max-depth exceeded"));
        }
        if self.frames.len() > self.options.max_expansions {
            return Err(self.fail(&call.node.loc, frame, "Expansion: max-expansions exceeded"));
        }
        self.schedule(
            &def.body,
            &Env {
                frame,
                captures: Rc::default(),
            },
            call.out,
        );
        Ok(())
    }
    /// Interpret one node without recursive calls. / 不使用递归调用解释一个节点。
    fn node(&mut self, node: &'a Node, env: Env, out: usize) -> Result<(), CompileError> {
        let (xml, text, kind) = match &node.kind {
            Kind::Text(value) => (escape(value, false), Some(Rc::from(value.as_str())), "text"),
            Kind::Insert(key) => {
                let value = self.get(key, &env, &node.loc)?;
                if !value.chars().all(|c| matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')) {
                    return Err(self.fail(&node.loc, env.frame, "Scalar: insert contains a character forbidden by XML 1.0"));
                }
                (escape(&value, false), Some(Rc::from(value)), "text")
            }
            Kind::Comment(value) => (format!("<!--{value}-->"), None, "comment"),
            Kind::Pi(value) => (format!("<?{value}?>"), None, "pi"),
            Kind::Element {
                name,
                attrs,
                children,
            } => {
                let mut xml = format!("<{name}");
                for (key, value) in attrs {
                    xml.push_str(&format!(" {key}=\"{}\"", escape(value, true)));
                }
                xml.push('>');
                self.tasks.push(Task::Close(node, env.clone(), out, name));
                self.schedule(children, &env, out);
                (xml, None, "start")
            }
            Kind::If { input, regex, body } => {
                let input = self.value(input, &env, &node.loc)?;
                if let Some(captures) = regex.captures(&input) {
                    let mut bindings = (*env.captures).clone();
                    for name in regex.capture_names().flatten() {
                        if let Some(value) = captures.name(name) {
                            bindings.insert(name.into(), value.as_str().into());
                        }
                    }
                    self.schedule(
                        body,
                        &Env {
                            frame: env.frame,
                            captures: Rc::new(bindings),
                        },
                        out,
                    );
                }
                return Ok(());
            }
            Kind::Slot { name, required } => {
                if let Some(tokens) = self.frames[env.frame].slots.get(name).cloned() {
                    for token in tokens.iter() {
                        self.emit_at(out, token.clone(), &node.loc, env.frame)?;
                    }
                } else if *required {
                    return Err(self.fail(
                        &node.loc,
                        env.frame,
                        format!("Signature: missing required fill '{name}'"),
                    ));
                }
                return Ok(());
            }
            Kind::Invoke {
                target,
                args,
                fills,
            } => {
                let target = match target {
                    Target::Source(path) => self.program.mains.get(path),
                    Target::Named(name) => self.program.symbols.get(name),
                }
                .copied()
                .ok_or_else(|| {
                    self.fail(&node.loc, env.frame, "Signature: unresolved macro target")
                })?;
                self.tasks.push(Task::Arg(
                    Box::new(Call {
                        target,
                        node,
                        env,
                        out,
                        args,
                        fills,
                        values: BTreeMap::new(),
                        slots: BTreeMap::new(),
                    }),
                    0,
                ));
                return Ok(());
            }
        };
        self.emit(
            out,
            Token {
                xml: Rc::from(xml),
                text,
                loc: &node.loc,
                frame: env.frame,
                kind,
            },
        )
    }
    /// Execute argument/fill continuations in strict source order. / 严格按源码顺序执行参数与填充续体。
    fn run(&mut self) -> Result<(), CompileError> {
        while let Some(task) = self.tasks.pop() {
            match task {
                Task::Node(node, env, out) => self.node(node, env, out)?,
                Task::Close(node, env, out, name) => self.emit(
                    out,
                    Token {
                        xml: Rc::from(format!("</{name}>")),
                        text: None,
                        loc: &node.loc,
                        frame: env.frame,
                        kind: "end",
                    },
                )?,
                Task::Arg(mut call, index) => {
                    let Some(arg) = call.args.get(index) else {
                        self.tasks.push(Task::Fill(call, 0));
                        continue;
                    };
                    if call.values.contains_key(&arg.name) {
                        return Err(self.fail(
                            &arg.loc,
                            call.env.frame,
                            format!("Signature: duplicate argument '{}'", arg.name),
                        ));
                    }
                    if let Value::Body(body) = &arg.value {
                        let buffer = self.buffer();
                        let env = call.env.clone();
                        self.tasks.push(Task::ArgDone(call, index, buffer));
                        self.schedule(body, &env, buffer);
                    } else {
                        let value = self.value(&arg.value, &call.env, &arg.loc)?;
                        call.values.insert(arg.name.clone(), value);
                        self.tasks.push(Task::Arg(call, index + 1));
                    }
                }
                Task::ArgDone(mut call, index, buffer) => {
                    let tokens = self.buffers.remove(&buffer).unwrap().tokens;
                    let mut value = String::new();
                    for token in tokens {
                        let Some(text) = token.text else {
                            return Err(self.fail(
                                token.loc,
                                token.frame,
                                "Scalar: scalar argument expansion produced a non-text node",
                            ));
                        };
                        value.push_str(&text);
                    }
                    call.values.insert(call.args[index].name.clone(), value);
                    self.tasks.push(Task::Arg(call, index + 1));
                }
                Task::Fill(call, index) => {
                    let Some(fill) = call.fills.get(index) else {
                        self.enter(*call)?;
                        continue;
                    };
                    if call.slots.contains_key(&fill.name) {
                        return Err(self.fail(
                            &fill.loc,
                            call.env.frame,
                            format!("Signature: duplicate fill '{}'", fill.name),
                        ));
                    }
                    let buffer = self.buffer();
                    let env = call.env.clone();
                    self.tasks.push(Task::FillDone(call, index, buffer));
                    self.schedule(&fill.body, &env, buffer);
                }
                Task::FillDone(mut call, index, buffer) => {
                    let tokens = self.buffers.remove(&buffer).unwrap().tokens;
                    call.slots
                        .insert(call.fills[index].name.clone(), Rc::new(tokens));
                    self.tasks.push(Task::Fill(call, index + 1));
                }
            }
        }
        Ok(())
    }
    /// Serialize a namespace-independent event IR with complete frame records. / 序列化命名空间独立的事件 IR 与完整调用帧记录。
    fn intermediate(&self, tokens: &[Token<'a>]) -> Result<String, CompileError> {
        let mut xml =
            String::from("<ir:expansion xmlns:ir=\"urn:xmlsquish:provenance\"><ir:frames>");
        for (id, frame) in self.frames.iter().enumerate() {
            let def = &self.program.defs[frame.def];
            xml.push_str(&format!("<ir:frame id=\"{id}\" macro=\"{}\" parent=\"{}\" source=\"{}\" definition-start=\"{}\" definition-end=\"{}\" call-source=\"{}\" call-line=\"{}\" call-start=\"{}\" call-end=\"{}\" file-dir=\"{}\" file-name=\"{}\"/>", frame.def, frame.parent.map(|v| v.to_string()).unwrap_or_default(), escape(&file_uri(&def.loc.path).map_err(|e| self.fail(&def.loc, id, e))?, true), def.loc.start, def.loc.end, escape(&file_uri(&frame.call.path).map_err(|e| self.fail(frame.call, id, e))?, true), frame.call.line, frame.call.start, frame.call.end, escape(&directory_uri(&def.loc.path).map_err(|e| self.fail(&def.loc, id, e))?, true), escape(&def.loc.path.file_name().unwrap_or_default().to_string_lossy(), true)));
        }
        xml.push_str("</ir:frames><ir:nodes>");
        for token in tokens {
            xml.push_str(&format!("<ir:node kind=\"{}\" frame=\"{}\" source=\"{}\" line=\"{}\" start=\"{}\" end=\"{}\">{}</ir:node>", token.kind, token.frame, escape(&file_uri(&token.loc.path).map_err(|e| self.fail(token.loc, token.frame, e))?, true), token.loc.line, token.loc.start, token.loc.end, escape(&token.xml, false)));
        }
        xml.push_str("</ir:nodes></ir:expansion>");
        Ok(xml)
    }
}
/// Expand a frozen program with explicit stacks and atomic success. / 用显式栈展开冻结程序，仅完整成功返回结果。
pub(super) fn expand(
    program: &Program,
    options: &CompileOptions,
) -> Result<CompileResult, CompileError> {
    let root = &program.defs[program.root];
    let mut machine = Machine {
        program,
        options,
        frames: vec![Frame {
            def: program.root,
            parent: None,
            call: &root.loc,
            depth: 1,
            args: options.args.clone(),
            slots: BTreeMap::new(),
        }],
        tasks: Vec::new(),
        buffers: BTreeMap::from([(0, Buffer::default())]),
        next_buffer: 1,
    };
    if options.max_depth == 0 || options.max_expansions == 0 {
        return Err(machine.fail(
            &root.loc,
            0,
            "Expansion: root frame exceeds max-depth or max-expansions",
        ));
    }
    for param in &root.params {
        if !options.args.contains_key(param) {
            return Err(machine.fail(
                &root.loc,
                0,
                format!("Signature: missing argument '{param}'"),
            ));
        }
    }
    for param in options.args.keys() {
        if !root.params.contains(param) {
            return Err(machine.fail(
                &root.loc,
                0,
                format!("Signature: unknown argument '{param}'"),
            ));
        }
    }
    for (slot, required) in &root.slots {
        if *required {
            return Err(machine.fail(
                &root.loc,
                0,
                format!("Signature: missing required fill '{slot}'"),
            ));
        }
    }
    machine.schedule(
        &root.body,
        &Env {
            frame: 0,
            captures: Rc::default(),
        },
        0,
    );
    machine.run()?;
    let tokens = &machine.buffers[&0].tokens;
    let output: String = tokens.iter().map(|token| token.xml.as_ref()).collect();
    roxmltree::Document::parse(&output).map_err(|error| {
        machine.fail(
            &root.loc,
            0,
            format!("Output: final result is not a well-formed XML document: {error}"),
        )
    })?;
    let intermediate = machine.intermediate(tokens)?;
    Ok(CompileResult {
        output,
        intermediate,
        logs: Vec::new(),
        root_context: machine.fail(&root.loc, 0, ""),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Compile a self-contained module. / 编译自包含模块。
    fn compile(body: &str, options: CompileOptions) -> Result<CompileResult, CompileError> {
        let source = format!(
            "<xs:module xmlns:xs=\"https://xmlsquish.moesegfault.dev/ns\" xmlns:m=\"urn:test\">{body}</xs:module>"
        );
        Compiler::with_options(options).compile(
            std::path::Path::new("runtime-test.xml"),
            &source,
            |_| Err("unexpected load".into()),
        )
    }
    #[test]
    fn scalar_body_is_decoded_once_and_insert_is_escaped() {
        let result = compile(r#"<xs:macro name="m:echo"><xs:param name="s"/><xs:insert get="arg.s"/></xs:macro><root><xs:call ref="m:echo"><xs:arg name="s"> &lt;x&gt;&amp; </xs:arg></xs:call></root>"#, CompileOptions::default()).unwrap();
        let doc = roxmltree::Document::parse(&result.output).unwrap();
        assert_eq!(doc.root_element().text(), Some(" <x>& "));
        roxmltree::Document::parse(&result.intermediate).unwrap();
    }
    #[test]
    fn deep_recursion_uses_explicit_stack_and_reports_chain() {
        let options = CompileOptions {
            max_depth: 5000,
            max_expansions: 6000,
            ..CompileOptions::default()
        };
        let error = compile(r#"<xs:macro name="m:loop"><xs:call ref="m:loop"/></xs:macro><root><xs:call ref="m:loop"/></root>"#, options).unwrap_err();
        assert!(error.message.contains("max-depth"));
        assert!(error.message.contains("frame #5000"));
    }
    #[test]
    fn capture_scopes_restore_and_do_not_leak_to_siblings() {
        let result = compile(r#"<root><xs:ifr str="a" pattern="(?&lt;x&gt;a)"><xs:insert get="match.x"/><xs:ifr str="b" pattern="(?&lt;x&gt;b)"><xs:insert get="match.x"/></xs:ifr><xs:insert get="match.x"/></xs:ifr></root>"#, CompileOptions::default()).unwrap();
        assert_eq!(
            roxmltree::Document::parse(&result.output)
                .unwrap()
                .root_element()
                .text(),
            Some("aba")
        );
        let error = compile(
            r#"<root><xs:ifr str="a" pattern="(?&lt;x&gt;a)"/><xs:insert get="match.x"/></root>"#,
            CompileOptions::default(),
        )
        .unwrap_err();
        assert!(error.message.contains("undefined binding"));
    }
    #[test]
    fn scalar_body_rejects_comment_nodes() {
        let error = compile(r#"<xs:macro name="m:echo"><xs:param name="s"/><xs:insert get="arg.s"/></xs:macro><root><xs:call ref="m:echo"><xs:arg name="s"><!--not scalar--></xs:arg></xs:call></root>"#, CompileOptions::default()).unwrap_err();
        assert!(error.message.contains("non-text node"));
    }
    #[test]
    fn terminating_recursion_and_caller_evaluated_fill() {
        let options = CompileOptions {
            max_depth: 3000,
            args: BTreeMap::from([("s".into(), "a".repeat(1500))]),
            ..CompileOptions::default()
        };
        let result = compile(r#"<xs:param name="s"/><xs:macro name="m:loop"><xs:param name="s"/><xs:ifr get="arg.s" pattern="^a(?&lt;tail&gt;.*)$"><xs:call ref="m:loop"><xs:arg name="s" get="match.tail"/></xs:call></xs:ifr><xs:ifr get="arg.s" pattern="^$">done</xs:ifr></xs:macro><root><xs:call ref="m:loop"><xs:arg name="s" get="arg.s"/></xs:call></root>"#, options).unwrap();
        assert_eq!(
            roxmltree::Document::parse(&result.output)
                .unwrap()
                .root_element()
                .text(),
            Some("done")
        );
        let result = compile(r#"<xs:macro name="m:emit">once</xs:macro><xs:macro name="m:panel"><xs:slot name="content" required="true"/></xs:macro><root><xs:ifr str="caller" pattern="(?&lt;who&gt;.*)"><xs:call ref="m:panel"><xs:fill name="content"><xs:insert get="match.who"/><xs:call ref="m:emit"/></xs:fill></xs:call></xs:ifr></root>"#, CompileOptions::default()).unwrap();
        assert_eq!(
            roxmltree::Document::parse(&result.output)
                .unwrap()
                .root_element()
                .text(),
            Some("calleronce")
        );
        let ir = roxmltree::Document::parse(&result.intermediate).unwrap();
        assert_eq!(
            ir.descendants()
                .filter(|n| n.has_tag_name(("urn:xmlsquish:provenance", "frame")))
                .count(),
            3
        );
    }
    #[test]
    fn captures_do_not_cross_macro_frames() {
        let error = compile(r#"<xs:macro name="m:read"><xs:insert get="match.x"/></xs:macro><root><xs:ifr str="a" pattern="(?&lt;x&gt;a)"><xs:call ref="m:read"/></xs:ifr></root>"#, CompileOptions::default()).unwrap_err();
        assert!(error.message.contains("undefined binding 'match.x'"));
        assert!(error.message.contains("frame #1"));
    }
    #[test]
    fn slot_budget_failure_reports_execution_frame_not_origin() {
        let body = format!(
            r#"<xs:macro name="m:f">{}<xs:slot name="s"/></xs:macro><root><xs:call ref="m:f"><xs:fill name="s">{}</xs:fill></xs:call></root>"#,
            "a".repeat(100),
            "b".repeat(100)
        );
        let options = CompileOptions {
            max_output_bytes: 190,
            ..CompileOptions::default()
        };
        let error = compile(&body, options).unwrap_err();
        assert!(error.message.contains("max-output-bytes"));
        assert!(error.message.contains("frame #1"), "{}", error.message);
    }
    #[test]
    fn output_budget_counts_utf8_serialized_bytes() {
        let result = compile("<root>猫</root>", CompileOptions::default()).unwrap();
        let options = CompileOptions {
            max_output_bytes: result.output.len() - 1,
            ..CompileOptions::default()
        };
        assert!(
            compile("<root>猫</root>", options)
                .unwrap_err()
                .message
                .contains("max-output-bytes")
        );
    }
}
