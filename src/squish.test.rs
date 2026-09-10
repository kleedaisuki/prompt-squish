//! Module-local regression tests. / 紧邻模块的回归测试。
use super::*;

fn ok(input: &str) -> SquishOutput {
    squish(input).expect("input should be lexically complete")
}

#[test]
fn empty_and_whitespace_only_inputs_become_empty() {
    assert_eq!(
        ok(""),
        SquishOutput {
            output: String::new(),
            stats: WhitespaceStats::default()
        }
    );
    assert_eq!(
        ok(" \t\r\n"),
        SquishOutput {
            output: String::new(),
            stats: WhitespaceStats {
                recognized: 4,
                removed: 4,
                inserted: 0
            },
        }
    );
}

#[test]
fn joins_every_atom_pair_with_one_ascii_space() {
    let result = ok(" \n<root>\t hello\r\nworld </root>  ");
    assert_eq!(result.output, "<root> hello world </root>");
    assert_eq!(
        result.stats,
        WhitespaceStats {
            recognized: 9,
            removed: 6,
            inserted: 0
        }
    );
}

#[test]
fn adjacent_markup_and_text_are_still_separated() {
    let result = ok("<a><b>x</b></a>");
    assert_eq!(result.output, "<a> <b> x </b> </a>");
    assert_eq!(
        result.stats,
        WhitespaceStats {
            recognized: 0,
            removed: 0,
            inserted: 4
        }
    );
}

#[test]
fn preserves_markup_interior_byte_for_byte() {
    let input = "<node  a = \"x > y\"\n b='z'>value</node   >";
    assert_eq!(
        ok(input).output,
        "<node  a = \"x > y\"\n b='z'> value </node   >"
    );
}

#[test]
fn recognizes_only_xml_s_as_whitespace() {
    let result = ok("a\u{00a0}b\u{2003}c d");
    assert_eq!(result.output, "a\u{00a0}b\u{2003}c d");
    assert_eq!(
        result.stats,
        WhitespaceStats {
            recognized: 1,
            removed: 0,
            inserted: 0
        }
    );
}

#[test]
fn copies_comments_cdata_and_processing_instructions_as_atoms() {
    let input = "<!-- a > b --> <![CDATA[ <x>  ]]><?pi a > b?>";
    let result = ok(input);
    assert_eq!(
        result.output,
        "<!-- a > b --> <![CDATA[ <x>  ]]> <?pi a > b?>"
    );
    assert_eq!(
        result.stats,
        WhitespaceStats {
            recognized: input
                .as_bytes()
                .iter()
                .filter(|&&byte| is_xml_space(byte))
                .count(),
            removed: 0,
            inserted: 1
        }
    );
}

#[test]
fn doctype_handles_quotes_subset_depth_comments_and_pi() {
    let input = "<!DOCTYPE root [\n<!ENTITY gt '>'>\n<!-- ] > -->\n<?inside ] ?>\n<!ELEMENT root (#PCDATA)>\n]>\n<root>x</root>";
    let result = ok(input);
    let split = input.find("\n<root>").unwrap();
    assert_eq!(
        result.output,
        format!("{} <root> x </root>", &input[..split])
    );
    assert_eq!(
        result.stats.recognized,
        input
            .as_bytes()
            .iter()
            .filter(|&&byte| is_xml_space(byte))
            .count()
    );
    assert_eq!(result.stats.removed, 0);
    assert_eq!(result.stats.inserted, 2);
}

#[test]
fn doctype_keyword_requires_a_boundary() {
    assert_eq!(ok("<!DOCTYPEfoo>bar").output, "<!DOCTYPEfoo> bar");
    assert_eq!(ok("<!DOCTYPE><r/>").output, "<!DOCTYPE> <r/>");
    assert_eq!(
        squish("<!DOCTYPE"),
        Err(SquishError {
            kind: SquishErrorKind::Doctype,
            offset: 0
        })
    );
}

#[test]
fn unicode_text_is_sliced_on_utf8_boundaries_and_stats_are_char_counts() {
    let input = "  猫\t娘 <萌>✨</萌> ";
    let result = ok(input);
    assert_eq!(result.output, "猫 娘 <萌> ✨ </萌>");
    assert_eq!(
        result.stats,
        WhitespaceStats {
            recognized: 5,
            removed: 3,
            inserted: 2
        }
    );
    assert_eq!(
        result.output.chars().count(),
        input.chars().count() - result.stats.removed + result.stats.inserted
    );
}

#[test]
fn reports_each_unterminated_top_level_construct_at_its_byte_offset() {
    let cases = [
        ("猫 <!--no", SquishErrorKind::Comment, 4),
        ("x <![CDATA[no", SquishErrorKind::Cdata, 2),
        ("x <?no", SquishErrorKind::ProcessingInstruction, 2),
        ("x <!DOCTYPE root [", SquishErrorKind::Doctype, 2),
        ("x <tag attr='>'", SquishErrorKind::Tag, 2),
    ];
    for (input, kind, offset) in cases {
        assert_eq!(
            squish(input),
            Err(SquishError { kind, offset }),
            "{input:?}"
        );
    }
}

#[test]
fn reports_unterminated_nested_doctype_construct_at_nested_offset() {
    let comment = "<!DOCTYPE r [<!-- nope";
    assert_eq!(
        squish(comment),
        Err(SquishError {
            kind: SquishErrorKind::Comment,
            offset: 13
        })
    );
    let pi = "<!DOCTYPE r [<?nope";
    assert_eq!(
        squish(pi),
        Err(SquishError {
            kind: SquishErrorKind::ProcessingInstruction,
            offset: 13
        })
    );
}

#[test]
fn output_character_accounting_identity_always_holds_for_successes() {
    for input in [
        "",
        " ",
        "abc",
        "a b",
        "<a/>",
        "<a>x</a>",
        "\n<a  x=' '>\t猫\r</a>\n",
    ] {
        let result = ok(input);
        assert_eq!(
            result.output.chars().count(),
            input.chars().count() - result.stats.removed + result.stats.inserted,
            "{input:?}"
        );
    }
}

#[test]
fn distinguishes_reused_removed_and_inserted_whitespace() {
    assert_eq!(
        ok("a b").stats,
        WhitespaceStats {
            recognized: 1,
            removed: 0,
            inserted: 0
        }
    );
    let tab = ok("a\tb");
    assert_eq!(tab.output, "a b");
    assert_eq!(
        tab.stats,
        WhitespaceStats {
            recognized: 1,
            removed: 0,
            inserted: 0
        }
    );
    assert_eq!(
        ok("a  b").stats,
        WhitespaceStats {
            recognized: 2,
            removed: 1,
            inserted: 0
        }
    );
    assert_eq!(
        ok("<a><b>").stats,
        WhitespaceStats {
            recognized: 0,
            removed: 0,
            inserted: 1
        }
    );
}

#[test]
fn protected_markup_whitespace_is_recognized_but_not_removed() {
    let input = "<a  x=' \t'>";
    let result = ok(input);
    assert_eq!(result.output, input);
    assert_eq!(
        result.stats,
        WhitespaceStats {
            recognized: 4,
            removed: 0,
            inserted: 0
        }
    );
}
