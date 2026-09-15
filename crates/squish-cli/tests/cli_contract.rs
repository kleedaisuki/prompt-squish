//! 公共启动解析契约。 / Public bootstrap parsing contracts.

use clap::error::ErrorKind;
use squish_cli::{
    BootstrapOutcome, InspectSubject, MessageFormat, ParsedInvocation, QueryFormat, parse_from,
    parse_from_with_version,
};
use squish_protocol::{
    DependencySource, EmitKind, InspectView, LockMode, OperationLocation, OperationRequest,
    VcsChoice,
};

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
fn new_maps_only_creation_inputs_without_touching_the_filesystem() {
    let parsed = invocation([
        "xmlsquish",
        "new",
        "does/not/exist",
        "--name",
        "support_v2",
        "--vcs=none",
        "--message-format=json",
        "--config",
        "new.vcs=\"git\"",
    ]);
    let OperationRequest::New(request) = &parsed.request else {
        panic!("new must produce NewRequest")
    };
    assert_eq!(
        request.destination.as_path(),
        std::path::Path::new("does/not/exist")
    );
    assert_eq!(request.name.as_ref().unwrap().as_str(), "support_v2");
    assert_eq!(request.vcs, Some(VcsChoice::None));
    assert!(matches!(
        parsed.request.location(),
        OperationLocation::Prospective(destination)
            if destination.as_path() == std::path::Path::new("does/not/exist")
    ));
    assert_eq!(
        parsed.presentation.message_format,
        Some(MessageFormat::Json)
    );
    assert_eq!(parsed.config_overrides[0].key(), "new.vcs");
}

#[test]
fn new_defers_inferred_name_and_default_vcs_to_the_domain() {
    for path in ["hello", ".", ".."] {
        let parsed = invocation(["xmlsquish", "new", path]);
        let OperationRequest::New(request) = parsed.request else {
            panic!("new must produce NewRequest")
        };
        assert_eq!(request.destination.as_path(), std::path::Path::new(path));
        assert!(request.name.is_none());
        assert!(request.vcs.is_none());
    }
}

#[test]
fn new_rejects_malformed_explicit_name_and_non_contract_options_as_usage() {
    for name in ["", "bad.name", "café", "has space"] {
        let failure = parse_from(["xmlsquish", "new", "destination", "--name", name]).unwrap_err();
        assert_eq!(failure.exit_code(), 2, "{name:?}");
    }
    for option in ["--force", "--template", "--workspace", "--dry-run", "--yes"] {
        let failure = parse_from(["xmlsquish", "new", "destination", option]).unwrap_err();
        assert_eq!(failure.exit_code(), 2, "{option}");
    }
}

#[test]
fn help_exposes_exact_six_manager_commands() {
    let help = parse_from(["xmlsquish"])
        .unwrap()
        .into_invocation()
        .unwrap_err();
    for command in ["new", "build", "fmt", "add", "remove", "inspect"] {
        assert!(help.as_str().contains(command), "missing {command}");
    }
}

#[cfg(unix)]
#[test]
fn new_preserves_non_utf8_destination_for_domain_validation() {
    use std::os::unix::ffi::OsStringExt;

    let destination = std::ffi::OsString::from_vec(vec![b'b', b'a', b'd', 0xff]);
    let parsed = parse_from([
        std::ffi::OsString::from("xmlsquish"),
        std::ffi::OsString::from("new"),
        destination.clone(),
        std::ffi::OsString::from("--name=valid"),
    ])
    .unwrap()
    .into_invocation()
    .unwrap();
    let OperationRequest::New(request) = parsed.request else {
        panic!("new must produce NewRequest")
    };
    assert_eq!(request.destination.as_path().as_os_str(), destination);
}

#[test]
fn build_without_emit_requests_the_primary_prompt_artifact() {
    let parsed = invocation(["xmlsquish", "build"]);
    let OperationRequest::Build(request) = parsed.request else {
        panic!("build must produce BuildRequest")
    };
    assert_eq!(request.emit, vec![EmitKind::Prompt]);
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
fn inspect_artifact_path_is_carried_once_by_the_protocol_selector() {
    let parsed = invocation([
        "xmlsquish",
        "inspect",
        "artifact",
        "target/prompts/main.prompt",
        "--format=json",
    ]);
    let OperationRequest::Inspect(request) = parsed.request else {
        panic!("inspect must produce InspectRequest")
    };
    assert!(matches!(
        request.view,
        InspectView::Artifact(ref path) if path.as_str() == "target/prompts/main.prompt"
    ));
    assert!(parsed.execution.inspect_subject.is_none());
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

#[test]
fn composing_binary_injects_the_displayed_version() {
    let version = parse_from_with_version(["xmlsquish", "--version"], "9.8.7-root").unwrap_err();
    assert_eq!(version.kind(), ErrorKind::DisplayVersion);
    assert_eq!(version.exit_code(), 0);
    assert!(version.to_string().contains("xmlsquish 9.8.7-root"));
}

#[test]
fn convenience_and_versioned_entry_points_share_one_typed_parser() {
    let convenience = parse_from(["xmlsquish", "build", "--config", "build.jobs=3"])
        .unwrap()
        .into_invocation()
        .unwrap();
    let versioned =
        parse_from_with_version(["xmlsquish", "build", "--config", "build.jobs=3"], "9.8.7")
            .unwrap()
            .into_invocation()
            .unwrap();
    assert_eq!(convenience, versioned);
}
