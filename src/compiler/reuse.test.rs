//! Prepared-program reuse contracts. / 预编译程序复用契约。
use super::{CompileOptions, Compiler};
use std::{cell::Cell, collections::BTreeMap, path::Path};

const ENTRY: &str = r#"<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:reuse"><xs:import src="library.xml"/><xs:param name="x"/><R><xs:expand ref="m:echo"><xs:arg name="x" get="arg.x"/></xs:expand></R></xs:entry>"#;
const LIBRARY: &str = r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:reuse"><xs:macro name="m:echo"><xs:param name="x"/><xs:insert get="arg.x"/></xs:macro></xs:module>"#;

/// Construct explicit invocation-local scalar inputs. / 构造本次展开专属的显式标量输入。
fn options(value: &str) -> CompileOptions {
    CompileOptions {
        args: BTreeMap::from([("x".into(), value.into())]),
        ..Default::default()
    }
}

/// Preparation freezes sources, while every expansion owns its dynamic state.
/// 准备阶段冻结源码，每次展开独立拥有动态状态。
#[test]
fn prepared_program_reuses_sources_without_leaking_arguments_or_ir_identity() {
    let loads = Cell::new(0);
    let program = Compiler::default()
        .prepare(Path::new("reuse/entry.xml"), ENTRY, |_| {
            loads.set(loads.get() + 1);
            Ok(LIBRARY.to_owned())
        })
        .unwrap();
    assert_eq!(loads.get(), 1);
    let first = program.expand(&options("雪 & <猫>")).unwrap();
    let second = program.expand(&options("different")).unwrap();
    let repeated = program.expand(&options("雪 & <猫>")).unwrap();
    assert_ne!(first.output, second.output);
    assert_eq!(
        first, repeated,
        "output, provenance and diagnostics must be stable"
    );
    assert_eq!(loads.get(), 1, "expansion must not reload source files");
}

/// Failed invocations must not retain budgets, frames or argument environments.
/// 失败展开不得遗留预算计数、调用帧或参数环境。
#[test]
fn failed_expansion_does_not_poison_the_next_invocation() {
    let program = Compiler::default()
        .prepare(Path::new("reuse/entry.xml"), ENTRY, |_| Ok(LIBRARY.into()))
        .unwrap();
    let expected = program.expand(&options("ok")).unwrap();
    let mut limited = options("too deep");
    limited.max_depth = 1;
    assert!(program.expand(&limited).is_err());
    limited = options("too many");
    limited.max_expansions = 1;
    assert!(program.expand(&limited).is_err());
    limited = options("too large");
    limited.max_output_bytes = 1;
    assert!(program.expand(&limited).is_err());
    assert!(program.expand(&CompileOptions::default()).is_err());
    assert_eq!(expected, program.expand(&options("ok")).unwrap());
}

/// A new preparation observes edits; an existing snapshot never does.
/// 重新准备会观察源码修改，已有快照则不受影响。
#[test]
fn fresh_preparation_reads_changed_sources_but_old_snapshot_is_immutable() {
    let compiler = Compiler::default();
    let old = compiler
        .prepare(Path::new("reuse/entry.xml"), ENTRY, |_| Ok(LIBRARY.into()))
        .unwrap();
    let expected = old.expand(&options("value")).unwrap();
    let edited = LIBRARY.replace("<xs:insert get=\"arg.x\"/>", "changed");
    let new = compiler
        .prepare(Path::new("reuse/entry.xml"), ENTRY, |_| Ok(edited.clone()))
        .unwrap();
    assert_ne!(
        expected.output,
        new.expand(&options("value")).unwrap().output
    );
    assert_eq!(expected, old.expand(&options("value")).unwrap());
}
