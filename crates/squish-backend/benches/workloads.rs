//! Production public API probes with orthogonal payload, metadata and graph axes.

use crate::support;
use squish_backend::archive::{
    ArchiveEntry, ArchiveLimits, SopackPayload, read_reproducible_zip, read_sopack,
    write_reproducible_zip, write_sopack,
};
use squish_ir::*;
use std::{collections::BTreeMap, hint::black_box};

/// Timing and allocation modes share fixtures, but never instrumentation.
#[derive(Clone, Copy)]
pub enum Mode {
    /// System allocator, adaptive repeated batches.
    Latency,
    /// Tracking allocator, result destruction included in every sample.
    Allocations,
}

impl Mode {
    /// Invoke the common harness after fixture construction and assertions.
    fn measure<T>(
        self,
        workload: &str,
        mechanism: &str,
        dimensions: &[(&str, u64)],
        operation: impl FnMut() -> T,
    ) {
        match self {
            Self::Latency => {
                support::latency("archive", workload, mechanism, dimensions, operation)
            }
            Self::Allocations => {
                support::allocations("archive", workload, mechanism, dimensions, operation);
            }
        }
    }
}

/// Build all inputs outside operations; no operation performs filesystem access.
pub fn run(mode: Mode) {
    if support::smoke_requested() {
        zip_case(mode, "zip_smoke", 8, 64 * 1024);
        sopack_case(mode, "sopack_smoke", &library(4, 2, Topology::Cycle), 0);
        fanout_case(mode, 2);
        return;
    }
    for members in [1, 8, 64, 512, 4096] {
        zip_case(mode, "zip_member_count", members, 4 * 1024 * 1024);
    }
    for bytes in [
        0,
        4096,
        64 * 1024,
        1024 * 1024,
        4 * 1024 * 1024,
        16 * 1024 * 1024,
    ] {
        zip_case(mode, "zip_payload_bytes", 32, bytes);
    }
    for fanout in [1, 2, 8, 32, 128, 256] {
        fanout_case(mode, fanout);
    }
    for ir_bytes in [0, 4096, 64 * 1024, 1024 * 1024, 4 * 1024 * 1024] {
        let mut payload = library(1, 1, Topology::Isolated);
        if ir_bytes != 0 {
            payload
                .units
                .values_mut()
                .next()
                .unwrap()
                .header_mut()
                .semantic_strings = vec!["x".repeat(ir_bytes)];
        }
        sopack_case(mode, "sopack_ir_ownership_bytes", &payload, 0);
    }
    for topology in [
        Topology::Isolated,
        Topology::Chain,
        Topology::Star,
        Topology::Cycle,
    ] {
        for groups in [1, 2, 4, 8, 16, 32, 64, 128] {
            // U, source bytes and E remain identical across G within each topology.
            sopack_case(mode, topology.name(), &library(128, groups, topology), 0);
        }
    }
    for (workload, topology) in [
        ("provider_tail_512_isolated", Topology::Isolated),
        ("provider_tail_512_cycle", Topology::Cycle),
    ] {
        // A separately labeled tail: never conflate this growing U with fixed-U scaling.
        sopack_case(mode, workload, &library(512, 512, topology), 0);
    }
}

/// Split a fixed byte budget exactly, keeping all member names the same length.
fn zip_entries(members: usize, bytes: usize) -> Vec<ArchiveEntry> {
    (0..members)
        .rev()
        .map(|index| ArchiveEntry {
            path: format!("data/{index:06}.bin"),
            bytes: vec![index as u8; bytes / members + usize::from(index < bytes % members)],
        })
        .collect()
}

/// Reader includes its canonical writer rebuild; the writer is a cost control, not subtraction.
fn zip_case(mode: Mode, workload: &str, members: usize, bytes: usize) {
    let limits = ArchiveLimits::default();
    let entries = zip_entries(members, bytes);
    let encoded = write_reproducible_zip(&entries, limits).expect("valid ZIP fixture");
    let decoded = read_reproducible_zip(&encoded, limits).expect("strict ZIP fixture");
    let mut expected = entries.clone();
    expected.sort_by(|left, right| left.path.cmp(&right.path));
    assert_eq!(decoded, expected);
    assert_eq!(write_reproducible_zip(&decoded, limits).unwrap(), encoded);
    assert_eq!(
        decoded.iter().map(|entry| entry.bytes.len()).sum::<usize>(),
        bytes
    );
    let dimensions = [
        ("members", members as u64),
        ("content_bytes", bytes as u64),
        ("archive_bytes", encoded.len() as u64),
        (
            "name_bytes",
            entries.iter().map(|entry| entry.path.len() as u64).sum(),
        ),
    ];
    mode.measure(workload, "zip_write_borrowed_entries", &dimensions, || {
        write_reproducible_zip(black_box(&entries), limits).unwrap()
    });
    mode.measure(
        workload,
        "zip_strict_read_owned_members",
        &dimensions,
        || read_reproducible_zip(black_box(&encoded), limits).unwrap(),
    );
    mode.measure(
        workload,
        "zip_canonical_rebuild_control",
        &dimensions,
        || write_reproducible_zip(black_box(&decoded), limits).unwrap(),
    );
    mode.measure(workload, "zip_owned_member_deep_clone", &dimensions, || {
        black_box(&decoded).clone()
    });
}

/// Frozen graph shape; unit and source counts do not change when grouping changes.
#[derive(Clone, Copy)]
enum Topology {
    /// No imports: G graph rounds still visit every group.
    Isolated,
    /// A unit-level chain with U-1 bindings.
    Chain,
    /// Unit zero imports each other unit.
    Star,
    /// A unit-level cycle with U bindings.
    Cycle,
}

impl Topology {
    /// Stable raw-data label; topology comparisons remain distinct experiments.
    fn name(self) -> &'static str {
        match self {
            Self::Isolated => "provider_groups_isolated",
            Self::Chain => "provider_groups_chain",
            Self::Star => "provider_groups_star",
            Self::Cycle => "provider_groups_cycle",
        }
    }

    /// Bind targets without adding unused imports or hidden synthetic work.
    fn targets(self, index: usize, units: usize) -> Vec<usize> {
        match self {
            Self::Isolated => vec![],
            Self::Chain if index + 1 < units => vec![index + 1],
            Self::Star if index == 0 => (1..units).collect(),
            Self::Cycle => vec![(index + 1) % units],
            _ => vec![],
        }
    }
}

/// Build genuine valid module IR without charging frontend fixture construction.
fn module(source: &SourceKey, bytes: &[u8]) -> RelocatableUnitIr {
    let digest = SourceDigest::of(bytes);
    RelocatableUnitIr::Module(ModuleObject {
        header: UnitHeader {
            ir_schema: Version { major: 1, minor: 0 },
            language_abi: AbiId("dsl-0007".into()),
            frontend_abi: AbiId("xmlsquish.xml/2".into()),
            regex_abi: AbiId("regex-1.12".into()),
            source: source.clone(),
            imports: vec![],
            semantic_strings: vec![],
            qnames: vec![],
            regexes: vec![],
            feature_bits: FeatureBits(0),
        },
        definitions: vec![],
        external_symbols: vec![],
        interface: InterfaceSummary::default(),
        regions: vec![],
        ops: vec![],
        origins: OriginTable::default(),
        sources: SourceArchive {
            records: vec![SourceRecord {
                key: source.clone(),
                digest,
                bom_len: 0,
                exact_bytes: BlobRef {
                    digest: digest.0,
                    byte_len: bytes.len() as u64,
                },
                line_start_offsets: vec![0],
            }],
        },
        attachment: UnitSourceAttachment {
            source: source.clone(),
            source_digest: digest,
            source_record: SourceRef(0),
        },
        producer: Producer {
            tool_version: "bench-fixture".into(),
            build_fingerprint: "host-independent".into(),
        },
    })
}

/// Keep source paths/bytes/import slots fixed while changing exact provider grouping.
fn library(units: usize, groups: usize, topology: Topology) -> SopackPayload {
    assert!(groups > 0 && units % groups == 0);
    let keys: Vec<_> = (0..units)
        .map(|index| SourceKey::Project {
            package: PackageInstanceId {
                source_kind: 4,
                canonical_source: format!("fixture/group-{:04}", index % groups),
                package_name: "provider".into(),
                exact_revision: "1.0.0".into(),
            },
            path: vec!["src".into(), format!("unit-{index:04}.xml")],
        })
        .collect();
    let mut payload = SopackPayload {
        package_name: "archive-benchmark".into(),
        package_version: "1.0.0".into(),
        root_source: Some(keys[0].clone()),
        exports: BTreeMap::from([("main".into(), keys[0].clone())]),
        ..Default::default()
    };
    for (index, key) in keys.iter().enumerate() {
        // Exact 64-byte diagnostic attachments, valid regardless of unused XML semantics.
        let mut bytes = format!("<module id=\"{index:04}\"/>").into_bytes();
        bytes.resize(64, b' ');
        let mut unit = module(key, &bytes);
        for (slot, target) in topology.targets(index, units).into_iter().enumerate() {
            unit.header_mut().imports.push(ImportDecl {
                local_id: ImportId(slot as u32),
                expected_kind: UnitKind::Module,
                spec: ImportSpec::RelativeUri(format!("unit-{target:04}.xml")),
            });
            payload.imports.push(ImportBinding {
                importer: key.clone(),
                import: ImportId(slot as u32),
                target: keys[target].clone(),
            });
        }
        payload.sources.insert(key.clone(), bytes);
        payload.units.insert(key.clone(), unit);
    }
    payload
}

/// Shared on-disk bytes are constant while owner bindings/hash work/expanded bytes grow.
fn fanout_case(mode: Mode, fanout: usize) {
    let mut payload = library(1, 1, Topology::Isolated);
    let owner = payload.root_source.clone().unwrap();
    for index in 0..fanout {
        payload.assets.insert(
            (owner.clone(), format!("asset-{index:04}.bin")),
            vec![0xa5; 64 * 1024],
        );
    }
    sopack_case(mode, "shared_asset_binding_fanout", &payload, 64 * 1024);
}

/// Probes public normalization/transport and decouples deep ownership from write/read.
fn sopack_case(mode: Mode, workload: &str, payload: &SopackPayload, unique_asset_bytes: u64) {
    let limits = ArchiveLimits::default();
    let encoded = write_sopack(payload, limits).expect("valid SOPack fixture");
    let restored = read_sopack(&encoded, limits).expect("relocatable SOPack fixture");
    assert_eq!(restored.units.len(), payload.units.len());
    assert_eq!(restored.assets.len(), payload.assets.len());
    assert_eq!(restored.imports.len(), payload.imports.len());
    let mut expected_sources: Vec<_> = payload.sources.values().collect();
    let mut actual_sources: Vec<_> = restored.sources.values().collect();
    expected_sources.sort();
    actual_sources.sort();
    assert_eq!(actual_sources, expected_sources);
    // Source bytes are unique in these fixtures and survive relocation exactly.
    // Comparing edge endpoints by attachments checks topology, not merely counts.
    let edges = |library: &SopackPayload| {
        let mut edges: Vec<_> = library
            .imports
            .iter()
            .map(|binding| {
                (
                    library.sources[&binding.importer].clone(),
                    binding.import.0,
                    library.sources[&binding.target].clone(),
                )
            })
            .collect();
        edges.sort();
        edges
    };
    assert_eq!(edges(&restored), edges(payload));
    for ((_, name), bytes) in &payload.assets {
        assert!(
            restored
                .assets
                .iter()
                .any(|((_, restored_name), restored_bytes)| name == restored_name
                    && bytes == restored_bytes)
        );
    }
    // Group collapse on read changes namespace ownership: write(read(x)) need not be x
    // for multi-provider libraries, so reproducibility checks use identical producer IR.
    assert_eq!(write_sopack(payload, limits).unwrap(), encoded);
    let groups = payload
        .units
        .keys()
        .filter_map(|key| match key {
            SourceKey::Project { package, .. } => Some(package),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let dimensions = [
        ("units", payload.units.len() as u64),
        ("providers", groups as u64),
        ("graph_rounds", groups as u64),
        ("import_edges", payload.imports.len() as u64),
        (
            "source_bytes",
            payload
                .sources
                .values()
                .map(|bytes| bytes.len() as u64)
                .sum(),
        ),
        (
            "encoded_unit_bytes",
            payload
                .units
                .values()
                .map(|unit| encode_unit_container(unit).unwrap().len() as u64)
                .sum(),
        ),
        ("asset_bindings", payload.assets.len() as u64),
        ("unique_asset_bytes", unique_asset_bytes),
        (
            "expanded_asset_bytes",
            payload
                .assets
                .values()
                .map(|bytes| bytes.len() as u64)
                .sum(),
        ),
        ("archive_bytes", encoded.len() as u64),
        ("group_round_visits", (groups * groups) as u64),
        ("edge_round_visits", (groups * payload.imports.len()) as u64),
    ];
    mode.measure(
        workload,
        "sopack_write_normalize_and_transport",
        &dimensions,
        || write_sopack(black_box(payload), limits).unwrap(),
    );
    mode.measure(
        workload,
        "sopack_strict_read_relocate_and_expand",
        &dimensions,
        || read_sopack(black_box(&encoded), limits).unwrap(),
    );
    mode.measure(workload, "sopack_payload_deep_clone", &dimensions, || {
        black_box(payload).clone()
    });
}
