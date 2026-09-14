use crate::{DiagnosticCode, LosslessXml, StyleEdition, TokenKind, format};

fn formatted(source: &[u8]) -> Vec<u8> {
    format(source, StyleEdition::V1)
        .unwrap()
        .apply(source)
        .unwrap()
}

#[test]
fn tape_preserves_bom_crlf_unicode_and_special_constructs() {
    let source = b"\xef\xbb\xbf<?xml version='1.0'?>\r\n<r a = '&amp;'>\xe4\xb8\xad<!--x--><![CDATA[<&x]]><?go x?></r>\r\n";
    let tape = LosslessXml::parse(source).unwrap();
    assert_eq!(tape.source(), source);
    assert!(tape.has_bom());
    assert_eq!(tape.line_ending(), Some("\r\n"));
    assert!(
        tape.tokens()
            .iter()
            .any(|token| token.kind == TokenKind::Cdata)
    );
    for token in tape.tokens() {
        assert_eq!(tape.raw(token), &source[token.span.start..token.span.end]);
    }
}

#[test]
fn formatter_only_changes_markup_trivia() {
    let source = b"<r  a = \"&amp;\"\n b= 'x'> a  b <![CDATA[ z ]]> <!-- c --> <?p q?> </r   >";
    assert_eq!(
        formatted(source),
        b"<r a=\"&amp;\" b='x'> a  b <![CDATA[ z ]]> <!-- c --> <?p q?> </r>"
    );
}

#[test]
fn scalar_body_and_mixed_content_are_byte_stable() {
    let source = b"<xs:entry xmlns:xs = 'urn:x'><p>Hello <b x = '1'> Klee </b> !</p><xs:arg name = 'x'>  exact\n scalar  </xs:arg></xs:entry>";
    let output = formatted(source);
    for needle in [
        b"Hello ".as_slice(),
        b" Klee ",
        b" !",
        b"  exact\n scalar  ",
    ] {
        assert!(output.windows(needle.len()).any(|part| part == needle));
    }
}

#[test]
fn formatting_is_idempotent() {
    let once = formatted(b"<r\n a = '1' b=\"&quot;\" ><x /></r >");
    let twice = formatted(&once);
    assert_eq!(once, twice);
    assert!(!format(&once, StyleEdition::V1).unwrap().is_changed());
}

#[test]
fn scanner_handles_deep_documents_without_recursion() {
    let depth = 50_000;
    let mut source = Vec::with_capacity(depth * 7);
    for _ in 0..depth {
        source.extend_from_slice(b"<x>");
    }
    for _ in 0..depth {
        source.extend_from_slice(b"</x>");
    }
    assert_eq!(
        LosslessXml::parse(&source).unwrap().tokens().len(),
        depth * 2
    );
}

#[test]
fn invalid_or_incomplete_xml_returns_diagnostics() {
    for (source, code) in [
        (b"<a>".as_slice(), DiagnosticCode::UnbalancedElement),
        (b"<a></b>", DiagnosticCode::UnbalancedElement),
        (b"<a x='1></a>", DiagnosticCode::InvalidSyntax),
        (b"<a>&bad</a>", DiagnosticCode::InvalidSyntax),
        (b"<a>\0</a>", DiagnosticCode::InvalidSyntax),
    ] {
        assert_eq!(format(source, StyleEdition::V1).unwrap_err().code, code);
    }
}

#[test]
fn plan_rejects_even_same_length_changed_source() {
    let source = b"<a  x = '1'/>";
    let plan = format(source, StyleEdition::V1).unwrap();
    assert!(plan.apply(b"<b  x = '1'/>").is_err());
}

#[test]
fn randomized_trivia_is_idempotent_and_data_preserving() {
    let trivia = [" ", "  ", "\t", "\n", "\r\n", " \t\r\n"];
    let mut state = 0x5eed_u64;
    for _ in 0..2_000 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = trivia[(state as usize) % trivia.len()];
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let b = trivia[(state as usize) % trivia.len()];
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let c = trivia[(state as usize) % trivia.len()];
        let source = format!("<r{a}x{b}={c}'&amp;'>payload &lt; exact</r{a}>");
        let once = formatted(source.as_bytes());
        let twice = formatted(&once);
        assert_eq!(once, twice);
        assert!(
            once.windows(b"payload &lt; exact".len())
                .any(|part| part == b"payload &lt; exact")
        );
    }
}
