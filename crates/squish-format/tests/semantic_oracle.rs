//! 格式化器到 XML 前端的语义差分 oracle。 / Formatter-to-XML-frontend semantic differential oracle.
//!
//! 这里刻意比较前端产出的语义字段，而不是源摘要、span 或 provenance。这样测试既能发现
//! 格式化器改变程序含义的回归，也不会把预期的字节位置变化误报为语义变化。
//! This deliberately compares semantic frontend fields rather than source digests, spans, or
//! provenance. It catches formatter regressions that change program meaning without mistaking
//! expected byte-position changes for semantic changes.

use squish_format::{StyleEdition, format};
use squish_ir::{
    InterfaceSummary, LocalName, MacroDef, OpRecord, PackageInstanceId, Region, RegionId,
    RelocatableUnitIr, SymbolKey, UnitHeader, UnitKind,
};
use squish_source::{
    LogicalPath, PackageId, SnapshotBuilder, SourceBlob, SourceId, SourceLocator, SourceProvider,
};
use squish_xml_front::{DSL_NAMESPACE, FrontendSourceContext, compile};
use std::{fmt::Debug, io, sync::Arc};

/// 不受格式化位置变化影响的可重定位 IR 投影。 / Relocatable IR projection unaffected by formatting positions.
#[derive(Debug, Eq, PartialEq)]
struct SemanticIr {
    /// The source root remains semantic even when two kinds share their arena layout.
    kind: UnitKind,
    header: UnitHeader,
    required_params: Vec<LocalName>,
    root_region: Option<RegionId>,
    definitions: Vec<MacroDef>,
    external_symbols: Vec<SymbolKey>,
    interface: Option<InterfaceSummary>,
    regions: Vec<Region>,
    ops: Vec<OpRecord>,
}

impl From<RelocatableUnitIr> for SemanticIr {
    fn from(unit: RelocatableUnitIr) -> Self {
        Self {
            kind: unit.kind(),
            header: unit.header().clone(),
            required_params: unit.required_params().to_vec(),
            root_region: unit.root_region(),
            definitions: unit.definitions().to_vec(),
            external_symbols: unit.external_symbols().to_vec(),
            interface: match &unit {
                RelocatableUnitIr::Module(module) => Some(module.interface.clone()),
                RelocatableUnitIr::Sopack(root) => Some(root.module.interface.clone()),
                RelocatableUnitIr::Entry(_) | RelocatableUnitIr::Pack(_) => None,
            },
            regions: unit.regions().to_vec(),
            ops: unit.ops().to_vec(),
        }
    }
}

#[derive(Clone)]
struct Memory(Vec<u8>);

impl SourceProvider for Memory {
    fn read(&self, _: &SourceLocator) -> io::Result<Vec<u8>> {
        Ok(self.0.clone())
    }
}

/// 用稳定身份冻结测试源码。 / Freezes test source under a stable identity.
fn blob(source: &[u8]) -> Arc<SourceBlob> {
    let mut builder = SnapshotBuilder::new(Memory(source.to_vec()));
    builder
        .load(
            SourceId::new(
                PackageId::new("fixture").unwrap(),
                LogicalPath::new("src/main.xml").unwrap(),
            ),
            SourceLocator::file("unused"),
        )
        .unwrap()
}

/// 返回与测试源码身份匹配的已解析包上下文。 / Returns resolved package context matching the fixture identity.
fn context() -> FrontendSourceContext {
    FrontendSourceContext::new(PackageInstanceId {
        source_kind: 1,
        canonical_source: "workspace:fixture".into(),
        package_name: "fixture".into(),
        exact_revision: "manifest:fixture@1".into(),
    })
}

/// 编译源码并仅保留语义投影。 / Compiles source and retains only its semantic projection.
fn semantics(source: &[u8]) -> SemanticIr {
    compile(&blob(source), &context()).unwrap().unit.into()
}

/// 断言格式化前后具有相同语义，并验证幂等性。 / Asserts equal semantics across formatting and verifies idempotence.
fn assert_semantics_preserved(source: impl AsRef<[u8]> + Debug) {
    let source = source.as_ref();
    let plan = format(source, StyleEdition::V1).unwrap();
    let formatted = plan.apply(source).unwrap();

    assert_eq!(
        semantics(source),
        semantics(&formatted),
        "source: {source:?}"
    );
    assert_eq!(
        format(&formatted, StyleEdition::V1)
            .unwrap()
            .apply(&formatted)
            .unwrap(),
        formatted,
        "source: {source:?}"
    );
}

#[test]
fn preserves_all_current_dsl_operation_families_and_xml_value_forms() {
    let entry = format!(
        r#"<xs:entry  xmlns:xs = "{DSL_NAMESPACE}" xmlns:m = 'urn:macro'>
  <xs:import src = "pkg:common/main" />
  <xs:param name = 'input' />
  <R plain = "A&amp;B" numeric = '&#x4B;&#108;&#101;&#101;' xmlns:q = "urn:q" q:n = 'v'>
    literal &lt; text<![CDATA[<cdata>&raw]]><!--kept--><?step now?>
    <xs:insert get = "arg.input" />
    <xs:insert get = "file.uri" />
    <xs:ifr str = 'abc' pattern = '(?P&lt;letter&gt;a)'>literal branch</xs:ifr>
    <xs:ifr get = 'arg.input' pattern = '(?P&lt;whole&gt;.+)'><xs:insert get = 'match.whole'/></xs:ifr>
    <xs:expand ref = "m:render">
      <xs:arg name = 'literal' value = 'A&amp;B'/>
      <xs:arg name = 'binding' get = 'file.name'/>
      <xs:arg name = 'body'><B x = "&quot;">body</B></xs:arg>
      <xs:fill name = 'content'>filled<![CDATA[ data]]></xs:fill>
    </xs:expand>
  </R>
</xs:entry>"#
    );
    let module = format!(
        r#"<xs:module xmlns:xs = '{DSL_NAMESPACE}' xmlns:m = "urn:macro">
  <xs:import src = 'parts/common.xml'/>
  <xs:import src = "file:/absolute/module.xml" />
  <xs:macro name = 'm:render'>
    <xs:param name = "literal"/><xs:param name = 'binding'/><xs:param name = "body"/>
    <xs:slot name = 'content' required = "true"/>
    <Out a = 'x&amp;y'><xs:insert get = "arg.literal"/><xs:slot name = "optional"/></Out>
  </xs:macro>
</xs:module>"#
    );

    assert_semantics_preserved(entry);
    assert_semantics_preserved(module);
}

#[test]
fn archive_units_preserve_frontend_semantics() {
    for root in ["pack", "sopack"] {
        let declaration = if root == "sopack" {
            "<xs:macro name='m:asset'><xs:asset path='blob.bin' name='data/blob.bin'/></xs:macro>"
        } else {
            "<xs:include path='entry.xml' name='instructions.prompt'/>"
        };
        assert_semantics_preserved(format!(
            "<xs:{root} xmlns:xs = '{DSL_NAMESPACE}' xmlns:m = 'urn:macro'>\n  {declaration}\n  <xs:asset path = 'blob.bin' name = 'blob.bin'/>\n</xs:{root}>"
        ));
    }
}

#[test]
fn randomized_tag_trivia_preserves_frontend_semantics() {
    const TRIVIA: [&str; 6] = [" ", "  ", "\t", "\n", "\r\n", " \t\r\n"];
    let mut state = 0x51_5eed_cafe_u64;

    for _ in 0..512 {
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            TRIVIA[state as usize % TRIVIA.len()]
        };
        let (a, b, c, d, e, f, g, h, i, j) = (
            next(),
            next(),
            next(),
            next(),
            next(),
            next(),
            next(),
            next(),
            next(),
            next(),
        );
        let source = format!(
            "<xs:entry{a}xmlns:xs{b}={c}'{DSL_NAMESPACE}'{d}xmlns:m{e}={f}\"urn:m\"><R{g}a{h}={i}'A&amp;B'{j}/></xs:entry{a}>"
        );
        assert_semantics_preserved(source);
    }
}
