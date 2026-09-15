//! 权威 wire 值的稳定 JSON 投影。 / Stable JSON projection of authoritative wire values.

use crate::*;
use core::fmt::Write;

/// `inspect` 的 UTF-8 JSON 结果；它不参与对象身份。 / UTF-8 JSON output for `inspect`; never identity-bearing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonProjection(pub String);
/// 提供面向机器但非权威的 JSON view。 / Provides a machine-oriented, non-authoritative JSON view.
pub trait Inspect {
    fn inspect_json(&self) -> JsonProjection;
}

impl Inspect for Container {
    fn inspect_json(&self) -> JsonProjection {
        let mut s = String::from("{\"format\":\"xsir-inspect-v1\",\"schema\":{");
        let _ = write!(s, "\"major\":{},\"minor\":{}", self.major, self.minor);
        let _ = write!(s, "}},\"kind\":\"{:?}\",\"semanticDigest\":", self.kind);
        string(&mut s, &self.semantic_digest().0.hex());
        s.push_str(",\"objectDigest\":");
        string(&mut s, &self.object_digest().0.hex());
        s.push_str(",\"sections\":[");
        for (i, x) in self.sections.iter().enumerate() {
            if i > 0 {
                s.push(',')
            }
            let _ = write!(
                s,
                "{{\"tag\":{},\"flags\":{},\"byteLength\":{}}}",
                x.tag,
                x.flags.0,
                x.payload.len()
            );
        }
        s.push_str("]}");
        JsonProjection(s)
    }
}
impl Inspect for RelocatableUnitIr {
    fn inspect_json(&self) -> JsonProjection {
        let (h, kind, defs, regions, ops) = match self {
            Self::Module(x) => (
                &x.header,
                "module",
                x.definitions.len(),
                x.regions.len(),
                x.ops.len(),
            ),
            Self::Entry(x) => (&x.header, "entry", 0, x.regions.len(), x.ops.len()),
        };
        let mut s = String::from("{\"format\":\"xsir-inspect-v1\",\"kind\":");
        string(&mut s, kind);
        s.push_str(",\"source\":");
        source(&mut s, &h.source);
        let _ = write!(
            s,
            ",\"imports\":{},\"definitions\":{},\"regions\":{},\"operations\":{},\"features\":{}",
            h.imports.len(),
            defs,
            regions,
            ops,
            h.feature_bits.0
        );
        s.push('}');
        JsonProjection(s)
    }
}
impl Inspect for LinkedDocumentIr {
    fn inspect_json(&self) -> JsonProjection {
        let mut s =
            String::from("{\"format\":\"xsir-inspect-v1\",\"kind\":\"linked-document\",\"abi\":");
        string(&mut s, &self.document_abi.0);
        let _ = write!(
            s,
            ",\"regions\":{},\"items\":{},\"strings\":{},\"features\":{}}}",
            self.regions.len(),
            self.items.len(),
            self.strings.len(),
            self.feature_bits.0
        );
        JsonProjection(s)
    }
}
impl Inspect for ExpansionTrace {
    fn inspect_json(&self) -> JsonProjection {
        JsonProjection(format!(
            "{{\"format\":\"xsir-inspect-v1\",\"kind\":\"expansion-trace\",\"frames\":{},\"origins\":{},\"scalarValues\":{},\"sequences\":{}}}",
            self.frames.len(),
            self.origins.len(),
            self.scalar_values.len(),
            self.sequences.len()
        ))
    }
}
impl Inspect for LinkedImage {
    fn inspect_json(&self) -> JsonProjection {
        let mut s = String::from(
            "{\"format\":\"xsir-inspect-v1\",\"kind\":\"linked-image\",\"languageAbi\":",
        );
        string(&mut s, &self.language_abi.0);
        let _ = write!(
            s,
            ",\"units\":{},\"definitions\":{},\"symbols\":{},\"relocations\":{},\"features\":{}}}",
            self.units.len(),
            self.definitions.len(),
            self.link_map.symbols.len(),
            self.link_map.relocations.len(),
            self.feature_bits.0
        );
        JsonProjection(s)
    }
}
impl Inspect for ArtifactByteMap {
    fn inspect_json(&self) -> JsonProjection {
        let bytes = self
            .entries
            .last()
            .map_or(0, |entry| entry.output_range.end);
        JsonProjection(format!(
            "{{\"format\":\"xsir-inspect-v1\",\"kind\":\"artifact-map\",\"ranges\":{},\"coveredBytes\":{bytes}}}",
            self.entries.len()
        ))
    }
}
impl Inspect for DebugBundle {
    fn inspect_json(&self) -> JsonProjection {
        let mut s =
            String::from("{\"format\":\"xsir-inspect-v1\",\"kind\":\"debug-bundle\",\"schema\":{");
        let _ = write!(
            s,
            "\"major\":{},\"minor\":{}}},\"features\":{},\"artifact\":{{\"digest\":",
            self.schema.major, self.schema.minor, self.feature_bits.0
        );
        string(&mut s, &self.artifact.digest.0.hex());
        let _ = write!(
            s,
            ",\"byteLength\":{}}},\"documentDigest\":",
            self.artifact.byte_len
        );
        string(&mut s, &self.document_digest.0.hex());
        s.push_str(",\"linkedImageDigest\":");
        string(&mut s, &self.linked_image.0.hex());
        s.push_str(",\"expansionTraceDigest\":");
        string(&mut s, &self.expansion_trace_digest.0.hex());
        s.push_str(",\"linkTraceDigest\":");
        string(&mut s, &self.link_trace_digest.0.hex());
        let _ = write!(
            s,
            ",\"document\":{{\"regions\":{},\"items\":{}}},\"trace\":{{\"frames\":{},\"origins\":{}}},\"linkTrace\":{{\"imports\":{},\"symbols\":{},\"diagnostics\":{}}},\"artifactMapRanges\":{},\"sourceArchives\":{},\"staticOrigins\":{},\"sourceBlobs\":{}}}",
            self.document.regions.len(),
            self.document.items.len(),
            self.expansion_trace.frames.len(),
            self.expansion_trace.origins.len(),
            self.link_trace.imports.len(),
            self.link_trace.symbols.len(),
            self.link_trace.diagnostics.len(),
            self.artifact_map.entries.len(),
            self.source_archives.len(),
            self.source_archives
                .iter()
                .map(|archive| archive.origins.entries.len())
                .sum::<usize>(),
            self.source_blobs.len()
        );
        JsonProjection(s)
    }
}
fn source(out: &mut String, k: &SourceKey) {
    match k {
        SourceKey::AdHoc { uri } => {
            out.push_str("{\"type\":\"adhoc\",\"uri\":");
            string(out, uri);
            out.push('}')
        }
        SourceKey::Project { package, path } => {
            out.push_str("{\"type\":\"project\",\"package\":");
            string(out, &package.package_name);
            out.push_str(",\"revision\":");
            string(out, &package.exact_revision);
            out.push_str(",\"path\":[");
            for (i, p) in path.iter().enumerate() {
                if i > 0 {
                    out.push(',')
                }
                string(out, p)
            }
            out.push_str("]}")
        }
    }
}
fn string(out: &mut String, v: &str) {
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_escapes() {
        let mut s = String::new();
        string(&mut s, "a\n\"b");
        assert_eq!(s, "\"a\\n\\\"b\"")
    }
}
