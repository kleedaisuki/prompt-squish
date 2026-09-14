//! Adversarial conformance checks for the lossless scanner contract.

use squish_format::{LosslessXml, StyleEdition, format};

#[test]
fn rejects_processing_instruction_without_a_target() {
    assert!(LosslessXml::parse(b"<? ?><r/>").is_err());
}

#[test]
fn rejects_reserved_xml_processing_instruction_target() {
    assert!(LosslessXml::parse(b"<r><?XML data?></r>").is_err());
}

#[test]
fn rejects_comment_whose_content_ends_in_hyphen() {
    assert!(LosslessXml::parse(b"<!--x---><r/>").is_err());
}

#[test]
fn rejects_name_characters_outside_the_xml_name_ranges() {
    let source = "<\u{100000} />";
    assert!(LosslessXml::parse(source.as_bytes()).is_err());
}

#[test]
fn records_the_first_observed_line_ending() {
    let source = b"<r>\ntext\r\n</r>";
    assert_eq!(
        LosslessXml::parse(source).unwrap().line_ending(),
        Some("\n")
    );
}

#[test]
fn records_a_bare_carriage_return_line_ending() {
    let source = b"<r>\rtext</r>";
    assert_eq!(
        LosslessXml::parse(source).unwrap().line_ending(),
        Some("\r")
    );
}

#[test]
fn rejects_malformed_xml_declaration() {
    assert!(LosslessXml::parse(b"<?xml garbage?><r/>").is_err());
}

#[test]
fn rejects_doctype_without_a_root_name() {
    assert!(LosslessXml::parse(b"<!DOCTYPE ><r/>").is_err());
}

#[test]
fn every_reported_edit_is_existing_tag_internal_xml_whitespace() {
    let source = b"\xef\xbb\xbf<?xml version='1.0'?><r\r\n a \t=\n'&amp;'  b=\"x\"> body\r\n<![CDATA[ z ]]> <!-- c --> </r \t>";
    let plan = format(source, StyleEdition::V1).unwrap();

    for edit in plan.edits() {
        assert!(!edit.expected.is_empty());
        assert!(
            edit.expected
                .iter()
                .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        );
        assert!(matches!(edit.replacement.as_slice(), b"" | b" "));
        assert_eq!(&source[edit.span.start..edit.span.end], edit.expected);
    }
}
