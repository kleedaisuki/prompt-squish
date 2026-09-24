//! 结构化 Document IR 的 golden/regression tests。 / Golden/regression tests over structured Document IR.

use super::*;
use squish_ir::{
    Digest, DocumentRegion, DocumentRegionId, ExpandedName, FrameId, FrameIdentity, FrameRecord,
    LinkedOpRef, ObjectDigest, OpId, OriginId, QNameId, QualifiedOriginRef, StringId, TraceRef,
    Version,
};

fn object() -> ObjectDigest {
    ObjectDigest(Digest::sha256("test-object", b"fixture"))
}

fn request(strings: Vec<&str>, qnames: Vec<&str>, items: Vec<DocumentItem>) -> BackendRequest {
    let count = items.len();
    let trace_ref = TraceRef {
        producer_op: LinkedOpRef {
            unit_slot: 0,
            op: OpId(0),
        },
        frame: FrameId(0),
        definition_origin: QualifiedOriginRef {
            object: object(),
            local: OriginId(0),
        },
        call_origin: None,
        substitution_chain: Vec::new(),
    };
    BackendRequest {
        document: LinkedDocumentIr {
            schema: Version { major: 1, minor: 0 },
            document_abi: AbiId(DOCUMENT_ABI.into()),
            root: DocumentRegionId(0),
            regions: vec![DocumentRegion {
                id: DocumentRegionId(0),
                start: 0,
                end: count as u32,
            }],
            items,
            strings: strings.into_iter().map(str::to_owned).collect(),
            qnames: qnames
                .into_iter()
                .map(|local_name| ExpandedName {
                    namespace_uri: "urn:test".into(),
                    local_name: local_name.into(),
                })
                .collect(),
            feature_bits: FeatureBits(0),
        },
        trace: ExpansionTrace {
            frames: vec![FrameRecord {
                id: FrameId(0),
                parent: None,
                identity: FrameIdentity::Entry,
                call_origin: None,
                definition_origin: QualifiedOriginRef {
                    object: object(),
                    local: OriginId(0),
                },
                depth: 1,
                args: Vec::new(),
                fills: Vec::new(),
            }],
            document_items: vec![trace_ref; count],
            ..Default::default()
        },
        options: SquishOptions::default(),
    }
}

fn nested_request(depth: usize) -> BackendRequest {
    let mut items = Vec::with_capacity(depth * 2);
    let mut regions = Vec::with_capacity(depth + 1);
    regions.push(DocumentRegion {
        id: DocumentRegionId(0),
        start: 0,
        end: (depth * 2) as u32,
    });
    for level in 0..depth {
        items.push(DocumentItem::ElementStart {
            expanded_name: QNameId(0),
            attributes: Vec::new(),
            children: DocumentRegionId((level + 1) as u32),
        });
        regions.push(DocumentRegion {
            id: DocumentRegionId((level + 1) as u32),
            start: (level + 1) as u32,
            end: (depth * 2 - level - 1) as u32,
        });
    }
    items.extend((0..depth).map(|_| DocumentItem::ElementEnd));
    let mut request = request(Vec::new(), vec!["R"], items);
    request.document.regions = regions;
    request
}

#[test]
fn structured_golden_preserves_current_lowering_and_whitespace() {
    let items = vec![
        DocumentItem::Comment { value: StringId(2) },
        DocumentItem::ProcessingInstruction {
            target: StringId(3),
            data: StringId(1),
        },
        DocumentItem::ElementStart {
            expanded_name: QNameId(0),
            attributes: vec![squish_ir::Attribute {
                name: QNameId(1),
                value: StringId(4),
            }],
            children: DocumentRegionId(1),
        },
        DocumentItem::Text { value: StringId(0) },
        DocumentItem::ElementStart {
            expanded_name: QNameId(2),
            attributes: Vec::new(),
            children: DocumentRegionId(2),
        },
        DocumentItem::Text { value: StringId(5) },
        DocumentItem::ElementEnd,
        DocumentItem::ElementEnd,
    ];
    let mut request = request(
        vec![
            "  hello\r\nworld ",
            "data",
            "keep",
            "user",
            "value",
            "猫\t娘 & < >",
        ],
        vec!["R", "attr", "子"],
        items,
    );
    request.document.regions.extend([
        DocumentRegion {
            id: DocumentRegionId(1),
            start: 3,
            end: 7,
        },
        DocumentRegion {
            id: DocumentRegionId(2),
            start: 5,
            end: 6,
        },
    ]);
    let output = SquishBackend.emit(request).unwrap();
    assert_eq!(
        String::from_utf8(output.bytes.clone()).unwrap(),
        "<!--keep--> <?user data?> <R> hello&#13; world <子> 猫 娘 &amp; &lt; &gt; </子> </R>"
    );
    assert!(
        output
            .byte_map
            .validate_against(&output.trace, output.bytes.len() as u64)
            .is_ok()
    );
    assert_eq!(output.cache_identity.backend_id, SQUISH_BACKEND_ID);
    assert!(
        output
            .trace
            .origins
            .iter()
            .any(|node| matches!(node, OriginNode::Synthetic { .. }))
    );
}

#[test]
fn root_validation_accepts_one_root_but_rejects_text() {
    let root = vec![
        DocumentItem::ElementStart {
            expanded_name: QNameId(0),
            attributes: Vec::new(),
            children: DocumentRegionId(1),
        },
        DocumentItem::ElementEnd,
    ];
    let mut valid = request(vec![], vec!["R"], root);
    valid.document.regions.push(DocumentRegion {
        id: DocumentRegionId(1),
        start: 1,
        end: 1,
    });
    assert!(SquishBackend.emit(valid).is_ok());

    let text = request(
        vec!["not whitespace"],
        vec![],
        vec![DocumentItem::Text { value: StringId(0) }],
    );
    assert_eq!(
        SquishBackend.emit(text).unwrap_err().kind,
        BackendErrorKind::InvalidRoot
    );
}

#[test]
fn comments_and_processing_instructions_must_remain_xml_serializable() {
    let bad_comment = request(
        vec!["bad--comment"],
        vec![],
        vec![DocumentItem::Comment { value: StringId(0) }],
    );
    assert_eq!(
        SquishBackend.emit(bad_comment).unwrap_err().kind,
        BackendErrorKind::InvalidXmlData
    );

    let bad_pi = request(
        vec!["data?>tail", "xml"],
        vec![],
        vec![DocumentItem::ProcessingInstruction {
            target: StringId(1),
            data: StringId(0),
        }],
    );
    assert_eq!(
        SquishBackend.emit(bad_pi).unwrap_err().kind,
        BackendErrorKind::InvalidXmlData
    );
}

#[test]
fn adjacent_text_occurrences_form_one_atom_but_keep_origin_segments() {
    let items = vec![
        DocumentItem::ElementStart {
            expanded_name: QNameId(0),
            attributes: Vec::new(),
            children: DocumentRegionId(1),
        },
        DocumentItem::Text { value: StringId(0) },
        DocumentItem::Text { value: StringId(1) },
        DocumentItem::ElementEnd,
    ];
    let mut request = request(vec!["a", "b"], vec!["R"], items);
    request.document.regions.push(DocumentRegion {
        id: DocumentRegionId(1),
        start: 1,
        end: 3,
    });
    let output = SquishBackend.emit(request).unwrap();
    assert_eq!(output.bytes, b"<R> ab </R>");
    let a = output
        .byte_map
        .entries
        .iter()
        .find(|entry| entry.output_range.start <= 4 && entry.output_range.end > 4)
        .unwrap();
    let b = output
        .byte_map
        .entries
        .iter()
        .find(|entry| entry.output_range.start <= 5 && entry.output_range.end > 5)
        .unwrap();
    assert_ne!(a.origin, b.origin);
    assert_eq!(output.metrics.whitespace_inserted, 2);
}

#[test]
fn markup_interior_whitespace_is_not_counted_as_squishable() {
    let items = vec![
        DocumentItem::Comment { value: StringId(0) },
        DocumentItem::ElementStart {
            expanded_name: QNameId(0),
            attributes: Vec::new(),
            children: DocumentRegionId(1),
        },
        DocumentItem::ProcessingInstruction {
            target: StringId(1),
            data: StringId(2),
        },
        DocumentItem::ElementEnd,
    ];
    let mut request = request(vec!["a \t b", "pi", "x \n y"], vec!["R"], items);
    request.document.regions.push(DocumentRegion {
        id: DocumentRegionId(1),
        start: 2,
        end: 3,
    });
    let output = SquishBackend.emit(request).unwrap();
    assert_eq!(output.metrics.whitespace_recognized, 0);
    assert_eq!(output.metrics.whitespace_removed, 0);
}

/// 核对输出分隔符与空白指标使用同一边界判定。
/// Checks that output separators and whitespace metrics share the same boundary decision.
#[test]
fn whitespace_metrics_follow_emitted_gaps() {
    let items = vec![
        DocumentItem::ElementStart {
            expanded_name: QNameId(0),
            attributes: Vec::new(),
            children: DocumentRegionId(1),
        },
        DocumentItem::Text { value: StringId(0) },
        DocumentItem::ElementEnd,
    ];
    let mut request = request(vec![" a \t b "], vec!["R"], items);
    request.document.regions.push(DocumentRegion {
        id: DocumentRegionId(1),
        start: 1,
        end: 2,
    });

    let output = SquishBackend.emit(request).unwrap();
    assert_eq!(output.bytes, b"<R> a b </R>");
    assert_eq!(output.metrics.whitespace_recognized, 5);
    assert_eq!(output.metrics.whitespace_removed, 2);
    assert_eq!(output.metrics.whitespace_inserted, 0);
}

#[test]
fn xml_ncname_ranges_and_xml_char_production_are_enforced() {
    let valid_name = "A\u{b7}\u{301}";
    assert!(SquishBackend.emit(nested_named_request(valid_name)).is_ok());
    assert!(SquishBackend.emit(nested_named_request("名")).is_ok());
    assert_eq!(
        SquishBackend
            .emit(nested_named_request("bad:name"))
            .unwrap_err()
            .kind,
        BackendErrorKind::InvalidXmlData
    );

    let items = vec![
        DocumentItem::ElementStart {
            expanded_name: QNameId(0),
            attributes: Vec::new(),
            children: DocumentRegionId(1),
        },
        DocumentItem::Text { value: StringId(0) },
        DocumentItem::ElementEnd,
    ];
    let mut illegal_char = request(vec!["bad\u{1}"], vec!["R"], items);
    illegal_char.document.regions.push(DocumentRegion {
        id: DocumentRegionId(1),
        start: 1,
        end: 2,
    });
    let error = SquishBackend.emit(illegal_char).unwrap_err();
    assert_eq!(error.kind, BackendErrorKind::InvalidXmlData);
    assert_eq!(error.item, Some(1));

    for data_item in [
        DocumentItem::Comment { value: StringId(0) },
        DocumentItem::ProcessingInstruction {
            target: StringId(1),
            data: StringId(0),
        },
    ] {
        let items = vec![
            DocumentItem::ElementStart {
                expanded_name: QNameId(0),
                attributes: Vec::new(),
                children: DocumentRegionId(1),
            },
            data_item,
            DocumentItem::ElementEnd,
        ];
        let mut illegal = request(vec!["\u{1}", "pi"], vec!["R"], items);
        illegal.document.regions.push(DocumentRegion {
            id: DocumentRegionId(1),
            start: 1,
            end: 2,
        });
        let error = SquishBackend.emit(illegal).unwrap_err();
        assert_eq!(error.kind, BackendErrorKind::InvalidXmlData);
        assert_eq!(error.item, Some(1));
    }
}

fn nested_named_request(name: &str) -> BackendRequest {
    let mut request = request(
        vec![],
        vec![name],
        vec![
            DocumentItem::ElementStart {
                expanded_name: QNameId(0),
                attributes: Vec::new(),
                children: DocumentRegionId(1),
            },
            DocumentItem::ElementEnd,
        ],
    );
    request.document.regions.push(DocumentRegion {
        id: DocumentRegionId(1),
        start: 1,
        end: 1,
    });
    request
}

#[test]
fn deep_documents_are_iterative_and_budget_is_exact() {
    let request = nested_request(10_000);
    let expected = 10_000 * 7 + (10_000 * 2 - 1);
    let mut too_small = request.clone();
    too_small.options.max_output_bytes = expected as u64 - 1;
    assert_eq!(
        SquishBackend.emit(too_small).unwrap_err().kind,
        BackendErrorKind::OutputBudgetExceeded
    );
    let output = SquishBackend.emit(request).unwrap();
    assert_eq!(output.bytes.len(), expected);
}

#[test]
fn negotiation_and_options_are_part_of_identity() {
    let backend = SquishBackend;
    assert_ne!(
        backend.cache_identity(SquishOptions {
            max_output_bytes: 1
        }),
        backend.cache_identity(SquishOptions {
            max_output_bytes: 2
        })
    );
    let mut unsupported = nested_request(1);
    unsupported.document.feature_bits = FeatureBits(1);
    assert_eq!(
        backend.emit(unsupported).unwrap_err().kind,
        BackendErrorKind::IncompatibleDocument
    );
}
