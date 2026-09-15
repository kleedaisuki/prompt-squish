//! 公共启动解析契约。 / Public bootstrap parsing contracts.

use clap::error::ErrorKind;
use squish_cli::{
    BootstrapOutcome, InspectSubject, MessageFormat, ParsedInvocation, QueryFormat, parse_from,
};
use squish_protocol::{DependencySource, InspectView, LockMode, OperationRequest};

fn invocation<const N: usize>(arguments: [&str; N]) -> ParsedInvocation {
    parse_from(arguments)
        .unwrap()
        .into_invocation()
        .expect("argv contains an operation command")
}

#[test]
fn operation_and_presentation_are_separate_contracts() {
    let parsed = invocation(["xmlsquish", "build", "--offline", "--message-format=short"]);
    let OperationRequest::Build(request) = parsed.request else {
        panic!("build must produce BuildRequest")
    };
    assert_eq!(request.lock, LockMode::Offline);
    assert_eq!(
        parsed.presentation.message_format,
        Some(MessageFormat::Short)
    );
}

#[test]
fn source_kind_conflicts_are_bootstrap_usage_errors() {
    let failure = parse_from([
        "xmlsquish",
        "add",
        "common",
        "--registry",
        "public",
        "--path",
        "../common",
    ])
    .unwrap_err();
    assert_eq!(failure.kind(), ErrorKind::ArgumentConflict);
    assert_eq!(failure.exit_code(), 2);
}

#[test]
fn git_without_explicit_reference_remains_typed() {
    let parsed = invocation([
        "xmlsquish",
        "add",
        "common",
        "--git",
        "https://example.invalid/common.git",
    ]);
    let OperationRequest::Add(request) = parsed.request else {
        panic!("add must produce AddRequest")
    };
    assert!(matches!(request.source, DependencySource::Git { .. }));
}

#[test]
fn inspect_subject_prevents_identifier_path_guessing() {
    let parsed = invocation([
        "xmlsquish",
        "inspect",
        "source",
        "src:main",
        "--format=json",
    ]);
    assert_eq!(parsed.presentation.query_format, Some(QueryFormat::Json));
    assert!(matches!(
        parsed.request,
        OperationRequest::Inspect(squish_protocol::InspectRequest {
            view: InspectView::Source(_),
            ..
        })
    ));
    assert!(matches!(
        parsed.execution.inspect_subject,
        Some(InspectSubject::Source(_))
    ));
}

#[test]
fn bare_invocation_is_help_not_a_usage_failure() {
    let outcome = parse_from(["xmlsquish"]).unwrap();
    assert!(matches!(outcome, BootstrapOutcome::BareHelp(_)));
}

#[test]
fn repeated_global_config_overrides_retain_position_and_toml_text() {
    let parsed = invocation([
        "xmlsquish",
        "--config",
        "build.jobs=2",
        "build",
        "--config",
        "term.message-format=\"short\"",
        "--config=source.cache-root='cache=local'",
    ]);
    assert_eq!(parsed.config_overrides.len(), 3);
    assert_eq!(parsed.config_overrides[0].key(), "build.jobs");
    assert_eq!(parsed.config_overrides[0].value(), "2");
    assert_eq!(
        parsed.config_overrides[1]
            .clone()
            .into_assignment()
            .as_str(),
        "term.message-format=\"short\""
    );
    assert_eq!(parsed.config_overrides[2].key(), "source.cache-root");
    assert_eq!(parsed.config_overrides[2].value(), "'cache=local'");
}

#[test]
fn config_shape_validation_stops_at_the_loader_boundary() {
    let parsed = invocation(["xmlsquish", "build", "--config", "build.jobs="]);
    assert_eq!(parsed.config_overrides[0].value(), "");

    for malformed in ["build.jobs", "=4", ".jobs=4", "build..jobs=4", "build.=4"] {
        let failure = parse_from(["xmlsquish", "build", "--config", malformed]).unwrap_err();
        assert_eq!(failure.kind(), ErrorKind::ValueValidation, "{malformed}");
        assert_eq!(failure.exit_code(), 2, "{malformed}");
    }
}

#[test]
fn config_does_not_change_existing_global_conflicts_or_help() {
    let conflict =
        parse_from(["xmlsquish", "--config", "build.jobs=2", "-q", "build", "-v"]).unwrap_err();
    assert_eq!(conflict.kind(), ErrorKind::ArgumentConflict);

    let help = parse_from(["xmlsquish", "--help"]).unwrap_err();
    assert_eq!(help.kind(), ErrorKind::DisplayHelp);
    assert_eq!(help.exit_code(), 0);
}
