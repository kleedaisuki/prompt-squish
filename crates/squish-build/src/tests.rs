use std::collections::BTreeMap;

use crate::*;
use squish_protocol::{ArtifactKind, DigestAlgorithm};

fn id(value: &str) -> ActionId {
    ActionId::new(value).unwrap()
}

#[test]
fn publication_types_reject_nonportable_or_ambiguous_text() {
    for invalid in [
        "",
        "../escape",
        "a\\b",
        "NUL",
        "name.",
        "a:b",
        ".squish-publish/x",
    ] {
        assert!(
            PublicationPath::new(invalid).is_err(),
            "accepted `{invalid}`"
        );
    }
    assert_eq!(
        PublicationPath::new("target/chat.prompt").unwrap().as_str(),
        "target/chat.prompt"
    );
    assert!(PublicationTargetId::new("").is_err());
    assert!(LogicalArtifactName::new("\n").is_err());
}

#[test]
fn generation_id_has_one_canonical_boundary_encoding() {
    let id = GenerationId::from_bytes([0xab; 32]);
    assert_eq!(GenerationId::from_hex(&id.to_hex()).unwrap(), id);
    assert!(GenerationId::from_hex(&"AB".repeat(32)).is_err());
    assert!(GenerationId::from_hex("ab").is_err());
}
fn digest(value: &str) -> ContentDigest {
    ContentDigest::new(
        DigestAlgorithm::Other("test".to_owned()),
        value.as_bytes().to_vec(),
    )
    .unwrap()
}

fn action(name: &str, dependencies: &[&str], resources: Resources) -> Action {
    Action {
        id: id(name),
        key: KeyRecipe::new(
            "test-epoch",
            vec![InputRef::Blob(digest(name))],
            BTreeMap::from([("mode".to_owned(), "test".to_owned())]),
        )
        .unwrap(),
        kind: ActionKind::Compile,
        class: ResourceClass::Cpu,
        resources,
        dependencies: dependencies.iter().map(|value| id(value)).collect(),
        outputs: vec![Output {
            name: OutputName::new("main").unwrap(),
            kind: ArtifactKind::BinaryIr,
        }],
    }
}

fn success(name: &str) -> ActionResult {
    ActionResult {
        outcome: Ok(()),
        outputs: vec![ProducedOutput {
            name: OutputName::new("main").unwrap(),
            kind: ArtifactKind::BinaryIr,
            digest: digest(name),
            size: name.len() as u64,
        }],
        events: Vec::new(),
    }
}

#[test]
fn plan_rejects_every_structural_ambiguity() {
    let a = action("a", &[], Resources::new(1, 0, 0));
    assert!(matches!(
        BuildPlan::new([a.clone(), a.clone()]),
        Err(PlanError::DuplicateId(_))
    ));

    let missing = action("b", &["absent"], Resources::new(1, 0, 0));
    assert!(matches!(
        BuildPlan::new([missing]),
        Err(PlanError::MissingDependency { .. })
    ));

    let cycle_a = action("a", &["b"], Resources::new(1, 0, 0));
    let cycle_b = action("b", &["a"], Resources::new(1, 0, 0));
    assert!(matches!(
        BuildPlan::new([cycle_a, cycle_b]),
        Err(PlanError::Cycle(_))
    ));

    let mut collision = action("b", &[], Resources::new(1, 0, 0));
    collision.outputs.push(collision.outputs[0].clone());
    assert!(matches!(
        BuildPlan::new([a, collision]),
        Err(PlanError::OutputCollision { .. })
    ));
}

#[test]
fn plan_reuses_canonical_reverse_edges_without_changing_cycle_detection() {
    let resources = Resources::new(1, 0, 0);
    let plan = BuildPlan::new([
        action("root", &[], resources),
        action("later", &["root"], resources),
        action("earlier", &["root", "root"], resources),
    ])
    .unwrap();
    assert_eq!(plan.dependents(&id("root")), &[id("earlier"), id("later")]);

    let error = BuildPlan::new([
        action("a", &["b"], resources),
        action("b", &["c"], resources),
        action("c", &["b"], resources),
    ])
    .unwrap_err();
    assert_eq!(error, PlanError::Cycle(vec![id("b"), id("c"), id("b")]));
}

#[test]
fn ready_order_is_independent_of_input_permutation() {
    let permutations = [
        ["c", "a", "b"],
        ["b", "c", "a"],
        ["a", "b", "c"],
        ["c", "b", "a"],
    ];
    for permutation in permutations {
        let actions = permutation.map(|name| action(name, &[], Resources::new(1, 0, 0)));
        let mut scheduler = Scheduler::new(
            BuildPlan::new(actions).unwrap(),
            Resources::new(3, 0, 0),
            true,
        )
        .unwrap();
        let order: Vec<_> = (0..3)
            .map(|_| scheduler.next_dispatch().unwrap().action.id.to_string())
            .collect();
        assert_eq!(order, ["a", "b", "c"]);
    }
}

#[test]
fn quotas_are_reserved_and_released_in_all_dimensions() {
    let actions = [
        action("a", &[], Resources::new(1, 1, 10)),
        action("b", &[], Resources::new(1, 1, 10)),
    ];
    let capacity = Resources::new(1, 1, 10);
    let mut scheduler = Scheduler::new(BuildPlan::new(actions).unwrap(), capacity, true).unwrap();
    let first = scheduler.next_dispatch().unwrap();
    assert_eq!(scheduler.available(), Resources::default());
    assert!(scheduler.next_dispatch().is_none());
    scheduler.complete(&first.action.id, success("a")).unwrap();
    assert_eq!(scheduler.available(), capacity);
    assert_eq!(scheduler.next_dispatch().unwrap().action.id, id("b"));
}

#[test]
fn keep_going_runs_independent_work_and_blocks_descendants() {
    let plan = BuildPlan::new([
        action("a", &[], Resources::new(1, 0, 0)),
        action("child", &["a"], Resources::new(1, 0, 0)),
        action("z", &[], Resources::new(1, 0, 0)),
    ])
    .unwrap();
    let mut scheduler = Scheduler::new(plan, Resources::new(1, 0, 0), true).unwrap();
    let failed = scheduler.next_dispatch().unwrap();
    scheduler
        .complete(
            &failed.action.id,
            ActionResult::failure("compile", "bad source"),
        )
        .unwrap();
    assert!(matches!(
        scheduler.state(&id("child")),
        Some(ActionState::Blocked { .. })
    ));
    assert_eq!(scheduler.next_dispatch().unwrap().action.id, id("z"));
}

#[test]
fn no_keep_going_cancels_independent_ready_work() {
    let plan = BuildPlan::new([
        action("a", &[], Resources::new(1, 0, 0)),
        action("z", &[], Resources::new(1, 0, 0)),
    ])
    .unwrap();
    let mut scheduler = Scheduler::new(plan, Resources::new(1, 0, 0), false).unwrap();
    let failed = scheduler.next_dispatch().unwrap();
    scheduler
        .complete(
            &failed.action.id,
            ActionResult::failure("compile", "bad source"),
        )
        .unwrap();
    assert!(matches!(
        scheduler.state(&id("z")),
        Some(ActionState::Cancelled)
    ));
    assert!(scheduler.is_finished());
}

#[test]
fn equal_materialized_keys_are_single_flighted() {
    let a = action("a", &[], Resources::new(1, 0, 0));
    let mut b = action("b", &[], Resources::new(1, 0, 0));
    b.key = a.key.clone();
    b.outputs = a.outputs.clone();
    let mut scheduler = Scheduler::new(
        BuildPlan::new([a, b]).unwrap(),
        Resources::new(2, 0, 0),
        true,
    )
    .unwrap();
    let leader = scheduler.next_dispatch().unwrap();
    assert!(scheduler.next_dispatch().is_none());
    assert_eq!(scheduler.available(), Resources::new(1, 0, 0));
    scheduler
        .complete(&leader.action.id, success("shared"))
        .unwrap();
    assert!(scheduler.is_finished());
    assert!(matches!(
        scheduler.state(&id("a")),
        Some(ActionState::Succeeded)
    ));
    assert!(matches!(
        scheduler.state(&id("b")),
        Some(ActionState::Succeeded)
    ));
}

#[test]
fn downstream_key_uses_artifact_digest_not_predecessor_recipe() {
    let reference = OutputRef {
        action: id("producer"),
        output: OutputName::new("main").unwrap(),
    };
    let recipe =
        KeyRecipe::new("epoch", vec![InputRef::Output(reference)], BTreeMap::new()).unwrap();
    let schema = [Output {
        name: OutputName::new("main").unwrap(),
        kind: ArtifactKind::Prompt,
    }];
    let first = recipe
        .materialize(&ActionKind::Link, &schema, |_| Some(digest("same-output")))
        .unwrap();
    let second = recipe
        .materialize(&ActionKind::Link, &schema, |_| Some(digest("same-output")))
        .unwrap();
    let changed = recipe
        .materialize(&ActionKind::Link, &schema, |_| {
            Some(digest("different-output"))
        })
        .unwrap();
    assert_eq!(first, second);
    assert_ne!(first, changed);
}

#[test]
fn key_encoding_separates_sections_and_variants() {
    let direct =
        KeyRecipe::new("epoch", vec![InputRef::Blob(digest("x"))], BTreeMap::new()).unwrap();
    let indirect = KeyRecipe::new("epoch", Vec::new(), BTreeMap::new()).unwrap();
    let schema = [Output {
        name: OutputName::new("main").unwrap(),
        kind: ArtifactKind::BinaryIr,
    }];
    assert_ne!(
        direct.materialize(&ActionKind::Compile, &schema, |_| None),
        indirect.materialize(&ActionKind::Compile, &schema, |_| None),
    );
    assert_ne!(
        direct.materialize(&ActionKind::Compile, &schema, |_| None),
        direct.materialize(&ActionKind::Backend, &schema, |_| None),
    );
    assert_ne!(
        direct.materialize(&ActionKind::CreateProject, &schema, |_| None),
        direct.materialize(&ActionKind::CommitTransaction, &schema, |_| None),
    );
    let two_outputs = [
        schema[0].clone(),
        Output {
            name: OutputName::new("debug").unwrap(),
            kind: ArtifactKind::DebugInfo,
        },
    ];
    assert_ne!(
        direct.materialize(&ActionKind::Compile, &schema, |_| None),
        direct.materialize(&ActionKind::Compile, &two_outputs, |_| None),
    );
}

#[test]
fn invalid_completion_has_no_effect_and_can_be_retried() {
    let plan = BuildPlan::new([action("a", &[], Resources::new(1, 0, 0))]).unwrap();
    let mut scheduler = Scheduler::new(plan, Resources::new(1, 0, 0), true).unwrap();
    let dispatch = scheduler.next_dispatch().unwrap();
    scheduler.drain_events();
    let invalid = ActionResult {
        outcome: Ok(()),
        outputs: Vec::new(),
        events: vec![ActionEvent::Message {
            code: "bad".to_owned(),
            message: "must not escape".to_owned(),
        }],
    };
    assert!(matches!(
        scheduler.complete(&dispatch.action.id, invalid),
        Err(CompletionError::OutputSchema { .. })
    ));
    assert!(scheduler.drain_events().is_empty());
    assert_eq!(scheduler.available(), Resources::default());
    scheduler
        .complete(&dispatch.action.id, success("a"))
        .unwrap();
    assert!(scheduler.is_finished());
}

#[test]
fn late_same_key_result_exposes_outputs_without_dispatch() {
    let a = action("a", &[], Resources::new(1, 0, 0));
    let mut gate = action("gate", &[], Resources::new(1, 0, 0));
    gate.outputs.clear();
    let mut late = action("late", &["gate"], Resources::new(1, 0, 0));
    late.key = a.key.clone();
    let plan = BuildPlan::new([a, gate, late]).unwrap();
    let mut scheduler = Scheduler::new(plan, Resources::new(2, 0, 0), true).unwrap();
    let first = scheduler.next_dispatch().unwrap();
    let second = scheduler.next_dispatch().unwrap();
    assert_eq!(first.action.id, id("a"));
    assert_eq!(second.action.id, id("gate"));
    scheduler
        .complete(&first.action.id, success("shared"))
        .unwrap();
    scheduler
        .complete(&second.action.id, ActionResult::success())
        .unwrap();
    assert!(scheduler.next_dispatch().is_none());
    assert_eq!(
        scheduler.completed_outputs(&id("late")).unwrap()[0].digest,
        digest("shared")
    );
    assert!(scheduler.drain_events().iter().any(|event| matches!(event,
        ScheduleEvent::ResultAvailable { action, source: ResultSource::Cache, .. } if action == &id("late")
    )));
}

#[test]
fn no_keep_going_does_not_reclassify_already_running_work() {
    let plan = BuildPlan::new([
        action("a", &[], Resources::new(1, 0, 0)),
        action("z", &[], Resources::new(1, 0, 0)),
        action("zz-child", &["z"], Resources::new(1, 0, 0)),
    ])
    .unwrap();
    let mut scheduler = Scheduler::new(plan, Resources::new(2, 0, 0), false).unwrap();
    let a = scheduler.next_dispatch().unwrap();
    let z = scheduler.next_dispatch().unwrap();
    scheduler
        .complete(&a.action.id, ActionResult::failure("compile", "bad"))
        .unwrap();
    assert!(matches!(
        scheduler.state(&z.action.id),
        Some(ActionState::Running)
    ));
    scheduler.complete(&z.action.id, success("z")).unwrap();
    assert!(matches!(
        scheduler.state(&id("zz-child")),
        Some(ActionState::Cancelled)
    ));
}

#[test]
fn blocked_state_names_the_direct_dependency_at_each_link() {
    let plan = BuildPlan::new([
        action("root", &[], Resources::new(1, 0, 0)),
        action("middle", &["root"], Resources::new(1, 0, 0)),
        action("leaf", &["middle"], Resources::new(1, 0, 0)),
    ])
    .unwrap();
    let mut scheduler = Scheduler::new(plan, Resources::new(1, 0, 0), true).unwrap();
    let root = scheduler.next_dispatch().unwrap();
    scheduler
        .complete(&root.action.id, ActionResult::failure("compile", "bad"))
        .unwrap();

    assert_eq!(
        scheduler.state(&id("middle")),
        Some(&ActionState::Blocked {
            dependency: id("root")
        })
    );
    assert_eq!(
        scheduler.state(&id("leaf")),
        Some(&ActionState::Blocked {
            dependency: id("middle")
        })
    );
}

#[test]
fn cancellation_stops_dispatch_until_running_leaders_acknowledge() {
    let a = action("a", &[], Resources::new(2, 0, 0));
    let mut alias = action("alias", &[], Resources::new(2, 0, 0));
    alias.key = a.key.clone();
    alias.outputs = a.outputs.clone();
    let queued = action("z", &[], Resources::new(1, 0, 0));
    let capacity = Resources::new(2, 0, 0);
    let plan = BuildPlan::new([a, alias, queued]).unwrap();
    let mut scheduler = Scheduler::new(plan, capacity, true).unwrap();

    let leader = scheduler.next_dispatch().unwrap();
    assert!(scheduler.next_dispatch().is_none());
    assert_eq!(scheduler.available(), Resources::default());

    assert_eq!(scheduler.request_cancellation(), vec![id("a")]);
    assert!(scheduler.cancellation_requested());
    assert!(scheduler.next_dispatch().is_none());
    assert_eq!(scheduler.state(&id("z")), Some(&ActionState::Cancelled));
    assert_eq!(scheduler.state(&id("a")), Some(&ActionState::Running));
    assert_eq!(scheduler.state(&id("alias")), Some(&ActionState::Running));
    assert!(!scheduler.is_finished());
    assert_eq!(scheduler.available(), Resources::default());

    scheduler.complete_cancelled(&leader.action.id).unwrap();
    assert_eq!(scheduler.state(&id("a")), Some(&ActionState::Cancelled));
    assert_eq!(scheduler.state(&id("alias")), Some(&ActionState::Cancelled));
    assert_eq!(scheduler.available(), capacity);
    assert!(scheduler.is_finished());
}

#[test]
fn impossible_resource_request_is_rejected_up_front() {
    let plan = BuildPlan::new([action("large", &[], Resources::new(2, 1, 100))]).unwrap();
    assert!(matches!(
        Scheduler::new(plan, Resources::new(1, 1, 100), true),
        Err(SchedulerError::Unschedulable { .. })
    ));
}

#[test]
fn plan_rejects_unavailable_named_input() {
    let producer = action("producer", &[], Resources::new(1, 0, 0));
    let mut consumer = action("consumer", &["producer"], Resources::new(1, 0, 0));
    consumer.key = KeyRecipe::new(
        "epoch",
        vec![InputRef::Output(OutputRef {
            action: id("producer"),
            output: OutputName::new("missing").unwrap(),
        })],
        BTreeMap::new(),
    )
    .unwrap();
    assert!(matches!(
        BuildPlan::new([producer, consumer]),
        Err(PlanError::InvalidInput { .. })
    ));
}

#[test]
fn early_cutoff_ignores_changed_predecessor_recipe_when_bytes_match() {
    fn consumer(producer: &str) -> Action {
        let mut value = action("consumer", &[producer], Resources::new(1, 0, 0));
        value.key = KeyRecipe::new(
            "consumer-epoch",
            vec![InputRef::Output(OutputRef {
                action: id(producer),
                output: OutputName::new("main").unwrap(),
            })],
            BTreeMap::new(),
        )
        .unwrap();
        value
    }
    fn consumer_key(producer_name: &str) -> ActionKey {
        let producer = action(producer_name, &[], Resources::new(1, 0, 0));
        let plan = BuildPlan::new([producer, consumer(producer_name)]).unwrap();
        let mut scheduler = Scheduler::new(plan, Resources::new(1, 0, 0), true).unwrap();
        let producer = scheduler.next_dispatch().unwrap();
        scheduler
            .complete(&producer.action.id, success("stable-bytes"))
            .unwrap();
        scheduler.next_dispatch().unwrap().key
    }
    assert_eq!(consumer_key("producer-v1"), consumer_key("producer-v2"));
}

#[test]
fn persistent_cache_completion_is_structured_and_releases_resources() {
    let plan = BuildPlan::new([action("a", &[], Resources::new(1, 0, 0))]).unwrap();
    let capacity = Resources::new(1, 0, 0);
    let mut scheduler = Scheduler::new(plan, capacity, true).unwrap();
    let dispatch = scheduler.next_dispatch().unwrap();
    scheduler.drain_events();
    scheduler
        .complete_cached(&dispatch.action.id, success("cached").outputs)
        .unwrap();
    assert_eq!(scheduler.available(), capacity);
    assert!(scheduler.drain_events().iter().any(|event| matches!(
        event,
        ScheduleEvent::ResultAvailable {
            source: ResultSource::Cache,
            ..
        }
    )));
}

#[test]
fn semantic_digest_is_deterministic_under_action_and_option_insertion_order() {
    let mut first = action("a", &[], Resources::new(1, 2, 3));
    first.key = KeyRecipe::new(
        "epoch",
        vec![InputRef::Blob(digest("input"))],
        BTreeMap::from([
            ("alpha".to_owned(), "1".to_owned()),
            ("beta".to_owned(), "2".to_owned()),
        ]),
    )
    .unwrap();
    let mut same = first.clone();
    let mut reversed_options = BTreeMap::new();
    reversed_options.insert("beta".to_owned(), "2".to_owned());
    reversed_options.insert("alpha".to_owned(), "1".to_owned());
    same.key = KeyRecipe::new(
        "epoch",
        vec![InputRef::Blob(digest("input"))],
        reversed_options,
    )
    .unwrap();
    let second = action("z", &[], Resources::new(4, 5, 6));

    let left = BuildPlan::new([first, second.clone()]).unwrap();
    let right = BuildPlan::new([second, same]).unwrap();
    assert_eq!(left.semantic_digest(), right.semantic_digest());
    assert_eq!(left.semantic_digest().algorithm(), "blake3");
    assert_eq!(left.semantic_digest().as_bytes().len(), 32);
}

#[test]
fn semantic_digest_canonicalizes_dependency_sets_and_removes_duplicates() {
    let parents = [
        action("a", &[], Resources::new(1, 0, 0)),
        action("b", &[], Resources::new(1, 0, 0)),
    ];
    let canonical = action("child", &["a", "b"], Resources::new(1, 0, 0));
    let repeated = action("child", &["b", "a", "b", "a"], Resources::new(1, 0, 0));
    let left = BuildPlan::new([parents[0].clone(), parents[1].clone(), canonical]).unwrap();
    let right = BuildPlan::new([parents[0].clone(), parents[1].clone(), repeated]).unwrap();

    assert_eq!(left.semantic_digest(), right.semantic_digest());
    assert_eq!(
        right.action(&id("child")).unwrap().dependencies,
        [id("a"), id("b")]
    );
}

#[test]
fn semantic_digest_is_sensitive_to_each_action_semantic_field() {
    let base = action("node", &[], Resources::new(1, 2, 3));
    let base_digest = BuildPlan::new([base.clone()]).unwrap().semantic_digest();
    let assert_changed = |candidate: Action, field: &str| {
        assert_ne!(
            base_digest,
            BuildPlan::new([candidate]).unwrap().semantic_digest(),
            "semantic field was omitted: {field}"
        );
    };

    let mut candidate = base.clone();
    candidate.id = id("renamed");
    assert_changed(candidate, "action id");
    let mut candidate = base.clone();
    candidate.kind = ActionKind::Link;
    assert_changed(candidate, "action kind");
    let mut candidate = base.clone();
    candidate.class = ResourceClass::Io;
    assert_changed(candidate, "resource class");
    for (resources, field) in [
        (Resources::new(9, 2, 3), "cpu resources"),
        (Resources::new(1, 9, 3), "io resources"),
        (Resources::new(1, 2, 9), "memory resources"),
    ] {
        let mut candidate = base.clone();
        candidate.resources = resources;
        assert_changed(candidate, field);
    }
    let mut candidate = base.clone();
    candidate.key = KeyRecipe::new(
        "changed-epoch",
        vec![InputRef::Blob(digest("node"))],
        BTreeMap::from([("mode".to_owned(), "test".to_owned())]),
    )
    .unwrap();
    assert_changed(candidate, "semantic epoch");
    let mut candidate = base.clone();
    candidate.key = KeyRecipe::new(
        "test-epoch",
        vec![InputRef::Blob(digest("changed-input"))],
        BTreeMap::from([("mode".to_owned(), "test".to_owned())]),
    )
    .unwrap();
    assert_changed(candidate, "typed input");
    let mut candidate = base.clone();
    candidate.key = KeyRecipe::new(
        "test-epoch",
        vec![InputRef::Blob(
            ContentDigest::new(
                DigestAlgorithm::Other("different-algorithm".to_owned()),
                b"node".to_vec(),
            )
            .unwrap(),
        )],
        BTreeMap::from([("mode".to_owned(), "test".to_owned())]),
    )
    .unwrap();
    assert_changed(candidate, "input digest algorithm");
    let mut candidate = base.clone();
    candidate.key = KeyRecipe::new(
        "test-epoch",
        vec![InputRef::Blob(digest("node"))],
        BTreeMap::from([("changed-option".to_owned(), "test".to_owned())]),
    )
    .unwrap();
    assert_changed(candidate, "option name");
    let mut candidate = base.clone();
    candidate.key = KeyRecipe::new(
        "test-epoch",
        vec![InputRef::Blob(digest("node"))],
        BTreeMap::from([("mode".to_owned(), "changed".to_owned())]),
    )
    .unwrap();
    assert_changed(candidate, "option value");
    let mut candidate = base.clone();
    candidate.outputs[0].name = OutputName::new("renamed").unwrap();
    assert_changed(candidate, "output name");
    let mut candidate = base.clone();
    candidate.outputs[0].kind = ArtifactKind::Prompt;
    assert_changed(candidate, "output kind");
    let mut candidate = base;
    candidate.outputs.push(Output {
        name: OutputName::new("debug").unwrap(),
        kind: ArtifactKind::DebugInfo,
    });
    assert_changed(candidate, "output schema");

    let mut ordered_outputs = action("ordered", &[], Resources::new(1, 0, 0));
    ordered_outputs.outputs.push(Output {
        name: OutputName::new("debug").unwrap(),
        kind: ArtifactKind::DebugInfo,
    });
    let mut reversed_outputs = ordered_outputs.clone();
    reversed_outputs.outputs.reverse();
    assert_ne!(
        BuildPlan::new([ordered_outputs]).unwrap().semantic_digest(),
        BuildPlan::new([reversed_outputs])
            .unwrap()
            .semantic_digest(),
        "semantic field was omitted: output order"
    );

    let parents = [
        action("a", &[], Resources::new(1, 0, 0)),
        action("b", &[], Resources::new(1, 0, 0)),
    ];
    let without_edge = action("child", &[], Resources::new(1, 0, 0));
    let with_edge = action("child", &["a"], Resources::new(1, 0, 0));
    assert_ne!(
        BuildPlan::new([parents[0].clone(), parents[1].clone(), without_edge])
            .unwrap()
            .semantic_digest(),
        BuildPlan::new([parents[0].clone(), parents[1].clone(), with_edge])
            .unwrap()
            .semantic_digest(),
        "semantic field was omitted: dependency set"
    );
}

#[test]
fn semantic_digest_preserves_input_order_type_and_output_ref_identity() {
    let blob_a = InputRef::Blob(digest("a"));
    let blob_b = InputRef::Blob(digest("b"));
    let mut ordered = action("node", &[], Resources::new(1, 0, 0));
    ordered.key = KeyRecipe::new(
        "epoch",
        vec![blob_a.clone(), blob_b.clone()],
        BTreeMap::new(),
    )
    .unwrap();
    let mut reversed = ordered.clone();
    reversed.key = KeyRecipe::new("epoch", vec![blob_b, blob_a], BTreeMap::new()).unwrap();
    assert_ne!(
        BuildPlan::new([ordered]).unwrap().semantic_digest(),
        BuildPlan::new([reversed]).unwrap().semantic_digest()
    );

    let producers = [
        action("producer-a", &[], Resources::new(1, 0, 0)),
        action("producer-b", &[], Resources::new(1, 0, 0)),
    ];
    let consumer = |producer: &str| {
        let mut value = action(
            "consumer",
            &["producer-a", "producer-b"],
            Resources::new(1, 0, 0),
        );
        value.key = KeyRecipe::new(
            "epoch",
            vec![InputRef::Output(OutputRef {
                action: id(producer),
                output: OutputName::new("main").unwrap(),
            })],
            BTreeMap::new(),
        )
        .unwrap();
        value
    };
    let plan = |producer| {
        BuildPlan::new([
            producers[0].clone(),
            producers[1].clone(),
            consumer(producer),
        ])
        .unwrap()
    };
    assert_ne!(
        plan("producer-a").semantic_digest(),
        plan("producer-b").semantic_digest(),
        "raw OutputRef identity must not be replaced by lossy/materialized metadata"
    );
}
