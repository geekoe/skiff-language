use super::support::*;

#[test]
fn receiver_effects_are_contextual_to_caller_reachable_values() {
    let model = AnalysisFixture::new(
        r#"
            function append(items: Array<string>) -> void {
              items.push("value")
            }

            function appendHop(items: Array<string>) -> void {
              append(items)
            }

            function callerOwned(items: Array<string>) -> void {
              appendHop(items)
            }

            function freshLocal() -> void {
              final items = Array.empty<string>()
              appendHop(items)
            }

            function freshLocalSuspend() -> void {
              final items = Array.empty<string>()
              std.time.sleep(Duration.milliseconds(1))
              appendHop(items)
            }
        "#,
    )
    .module("std.effect_test")
    .package(crate::shared::id::SKIFF_STD_PUBLICATION_ID)
    .analyze();

    for callable in ["append", "appendHop", "callerOwned"] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            no_effects(),
            "{callable}"
        );
    }
    assert_eq!(
        effects_in(&model, "std.effect_test", "freshLocal"),
        no_effects()
    );
    assert_eq!(
        effects_in(&model, "std.effect_test", "freshLocalSuspend"),
        pending_only_effects(vec![PendingEffectCategory::NativeCall])
    );
}

#[test]
fn json_object_set_effects_are_contextual_to_caller_reachable_values() {
    let model = AnalysisFixture::new(
        r#"
            function setCallerOwned(object: JsonObject) -> void {
              return object.set("value", 1)
            }

            function callerOwnedHop(object: JsonObject) -> void {
              return setCallerOwned(object)
            }

            function freshLocal() -> void {
              final object: JsonObject = {}
              return object.set("value", 1)
            }

            function freshLocalCallerValue(value: Json) -> void {
              final object: JsonObject = {}
              return object.set("value", value)
            }
        "#,
    )
    .module("std.effect_test")
    .package(crate::shared::id::SKIFF_STD_PUBLICATION_ID)
    .analyze();

    for callable in ["setCallerOwned", "callerOwnedHop"] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            no_effects(),
            "{callable}"
        );
        let CallableProvenanceSummary::Analyzed { return_origins, .. } =
            provenance_in(&model, "std.effect_test", callable)
        else {
            panic!("{callable} must keep exact constant-null provenance");
        };
        assert_eq!(return_origins, &vec![ValueProvenance::Constant]);
    }

    for callable in ["freshLocal", "freshLocalCallerValue"] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            no_effects(),
            "{callable}"
        );
        let CallableProvenanceSummary::Analyzed { return_origins, .. } =
            provenance_in(&model, "std.effect_test", callable)
        else {
            panic!("{callable} must keep exact constant-null provenance");
        };
        assert_eq!(return_origins, &vec![ValueProvenance::Constant]);
    }
}

#[test]
fn exact_date_and_duration_receiver_targets_use_sparse_semantics() {
    let model = AnalysisFixture::new(
        r#"
            function isBefore(left: Date, right: Date) -> bool {
              return left.isBefore(right)
            }

            function compare(left: Date, right: Date) -> integer {
              return left.compare(right)
            }

            function addMilliseconds(value: Date, delta: integer) -> Date {
              return value.addMilliseconds(delta)
            }

            function diffMilliseconds(left: Date, right: Date) -> integer {
              return left.diffMilliseconds(right)
            }

            function epochMilliseconds(value: Date) -> integer {
              return value.toEpochMilliseconds()
            }

            function durationMilliseconds(value: Duration) -> integer {
              return value.toMilliseconds()
            }
        "#,
    )
    .module("std.effect_test")
    .package(crate::shared::id::SKIFF_STD_PUBLICATION_ID)
    .analyze();

    for callable in [
        "isBefore",
        "compare",
        "addMilliseconds",
        "diffMilliseconds",
        "epochMilliseconds",
        "durationMilliseconds",
    ] {
        assert_eq!(
            effects_in(&model, "std.effect_test", callable),
            no_effects(),
            "{callable}"
        );
    }
    let receiver_targets = model
        .resolved_call_targets()
        .iter()
        .filter_map(|(_, target)| match target {
            ResolvedCallTarget::ReceiverBuiltin { op } => Some(op.canonical_key),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        receiver_targets,
        std::collections::BTreeSet::from([
            "receiver:Date.addMilliseconds@1",
            "receiver:Date.compare@1",
            "receiver:Date.diffMilliseconds@1",
            "receiver:Date.isBefore@1",
            "receiver:Date.toEpochMilliseconds@1",
            "receiver:Duration.toMilliseconds@1",
        ])
    );
}

#[test]
fn date_add_milliseconds_keeps_v1_proxy_expiry_detached() {
    let model = AnalysisFixture::new(
        r#"
            function upstreamRecoverAt(now: Date, delayMs: integer) -> Date {
              return now.addMilliseconds(delayMs)
            }

            function v1Proxy(now: Date, delayMs: integer) -> Date {
              return upstreamRecoverAt(now, delayMs)
            }
        "#,
    )
    .module("upstream_health")
    .package("skiff.run/codex-relay")
    .analyze();

    for callable in ["upstreamRecoverAt", "v1Proxy"] {
        assert_eq!(
            effects_in(&model, "upstream_health", callable),
            no_effects(),
            "{callable}"
        );
        assert!(matches!(
            provenance_in(&model, "upstream_health", callable),
            CallableProvenanceSummary::Analyzed { return_origins, .. }
                if return_origins == &vec![ValueProvenance::Fresh]
        ));
    }
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:Date.addMilliseconds@1"
        )
    }));
}

#[test]
fn date_diff_milliseconds_keeps_interaction_duration_shape_detached() {
    let model = AnalysisFixture::new(
        r#"
            function interactionDurationMs(startedAt: Date, completedAt: Date?) -> integer? {
              if completedAt == null {
                return null
              }
              return completedAt.diffMilliseconds(startedAt)
            }

            function adminLlmInteractionsList(
              startedAt: Date,
              completedAt: Date?
            ) -> integer? {
              return interactionDurationMs(startedAt, completedAt)
            }
        "#,
    )
    .module("interactions")
    .package("skiff.run/codex-relay")
    .analyze();

    for callable in ["interactionDurationMs", "adminLlmInteractionsList"] {
        assert_eq!(
            effects_in(&model, "interactions", callable),
            no_effects(),
            "{callable}"
        );
    }
    assert!(matches!(
        provenance_in(&model, "interactions", "interactionDurationMs"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins.contains(&ValueProvenance::Constant)
                && return_origins.contains(&ValueProvenance::Fresh)
                && !return_origins.iter().any(is_caller_parameter_provenance)
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:Date.diffMilliseconds@1"
        )
    }));
}

#[test]
fn nullable_date_compare_keeps_upstream_status_shape_detached() {
    let model = AnalysisFixture::new(
        r#"
            function upstreamStatus(now: Date, fixedRecoverAt: Date?) -> string {
              if fixedRecoverAt != null && now.compare(fixedRecoverAt) < 0 {
                return "recovering"
              }
              return "available"
            }
        "#,
    )
    .module("upstream_health")
    .package("skiff.run/codex-relay")
    .analyze();

    assert_eq!(
        effects_in(&model, "upstream_health", "upstreamStatus"),
        no_effects()
    );
    assert!(matches!(
        provenance_in(&model, "upstream_health", "upstreamStatus"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Constant]
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:Date.compare@1"
        )
    }));
}

#[test]
fn exact_string_contains_target_is_read_only_detached_and_non_suspending() {
    let model = AnalysisFixture::new(
        r#"
            function validEmail(value: string) -> bool {
              return value.contains("@")
            }
        "#,
    )
    .module("account")
    .package("skiff.run/account")
    .analyze();

    assert_eq!(effects_in(&model, "account", "validEmail"), no_effects());
    assert!(matches!(
        provenance_in(&model, "account", "validEmail"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Fresh]
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:string.contains@1"
        )
    }));
}

#[test]
fn exact_bytes_to_hex_target_is_read_only_detached_and_non_suspending() {
    let model = AnalysisFixture::new(
        r#"
            function encode(value: bytes) -> string {
              return value.toHex()
            }
        "#,
    )
    .module("raw_parser")
    .package("skiff.run/codex-relay")
    .analyze();

    assert_eq!(effects_in(&model, "raw_parser", "encode"), no_effects());
    assert!(matches!(
        provenance_in(&model, "raw_parser", "encode"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Fresh]
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:bytes.toHex@1"
        )
    }));
}

#[test]
fn exact_json_object_has_target_is_read_only_detached_and_non_suspending() {
    let model = AnalysisFixture::new(
        r#"
            function jsonObjectField(value: JsonObject, field: string) -> bool {
              return value.has(field)
            }

            function verifyDomainChallenge(value: JsonObject) -> bool {
              return jsonObjectField(value, "Status")
            }
        "#,
    )
    .module("account")
    .package("skiff.run/account")
    .analyze();

    for callable in ["jsonObjectField", "verifyDomainChallenge"] {
        assert_eq!(effects_in(&model, "account", callable), no_effects());
        assert!(matches!(
            provenance_in(&model, "account", callable),
            CallableProvenanceSummary::Analyzed { return_origins, .. }
                if return_origins == &vec![ValueProvenance::Fresh]
        ));
    }
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:JsonObject.has@1"
        )
    }));
}

#[test]
fn exact_json_object_delete_mutates_caller_receiver_but_discharges_fresh_receiver() {
    let model = AnalysisFixture::new(
        r#"
            function deleteCallerField(value: JsonObject, field: string) -> bool {
              return value.delete(field)
            }

            function sanitize() -> bool {
              final value: JsonObject = { instructions: "drop", keep: true }
              return value.delete("instructions")
            }
        "#,
    )
    .module("responses_projection")
    .package("skiff.run/codex-relay")
    .analyze();

    assert_eq!(effects_in(&model, "responses_projection", "deleteCallerField"), no_effects());
    assert_eq!(
        effects_in(&model, "responses_projection", "sanitize"),
        no_effects()
    );
    for callable in ["deleteCallerField", "sanitize"] {
        assert!(matches!(
            provenance_in(&model, "responses_projection", callable),
            CallableProvenanceSummary::Analyzed { return_origins, .. }
                if return_origins == &vec![ValueProvenance::Constant]
        ));
    }
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:JsonObject.delete@1"
        )
    }));
}

#[test]
fn json_object_delete_semantics_do_not_generalize_to_map_delete() {
    let model = AnalysisFixture::new(
        r#"
            function remove(value: Map<string, string>, key: string) -> bool {
              return value.delete(key)
            }
        "#,
    )
    .module("map_delete")
    .package("skiff.run/map-delete")
    .analyze();

    assert_eq!(effects_in(&model, "map_delete", "remove"), all_effects());
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:Map.delete@1"
        )
    }));
}

#[test]
fn exact_json_object_get_preserves_nested_alias_but_fresh_codec_shape_is_detached() {
    let model = AnalysisFixture::new(
        r#"
            function direct(value: JsonObject, key: string) -> Json {
              return value.get(key)
            }

            function jsonObject(value: Json?) -> JsonObject? {
              if value == null { return null }
              final parsed = catch<std.json.DecodeError>(
                std.json.decode<JsonObject>(std.json.encode<Json>(value))
              )
              if parsed.tag == "ok" { return parsed.value }
              return null
            }

            function jsonField(value: Json?, key: string) -> Json? {
              final object = jsonObject(value)
              if object == null { return null }
              return object.get(key)
            }

            function claimsFromJwt(payload: Json?) -> Json? {
              return jsonField(payload, "https://api.openai.com/profile")
            }
        "#,
    )
    .module("chatgpt_plan.codec")
    .package("skiff.run/llm-providers")
    .analyze();

    assert_eq!(effects_in(&model, "chatgpt_plan.codec", "direct"), no_effects());
    assert!(matches!(
        provenance_in(&model, "chatgpt_plan.codec", "direct"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![caller_container_projection(0)]
    ));

    for callable in ["jsonObject", "jsonField", "claimsFromJwt"] {
        assert_eq!(
            effects_in(&model, "chatgpt_plan.codec", callable),
            no_effects(),
            "{callable}"
        );
        let CallableProvenanceSummary::Analyzed { return_origins, .. } =
            provenance_in(&model, "chatgpt_plan.codec", callable)
        else {
            panic!("{callable} must keep analyzed detached provenance")
        };
        assert!(
            return_origins.contains(&ValueProvenance::Fresh),
            "{callable}: {return_origins:?}"
        );
        assert!(
            !return_origins.iter().any(is_caller_parameter_provenance),
            "{callable}: {return_origins:?}"
        );
    }
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:JsonObject.get@1"
        )
    }));
}

#[test]
fn exact_map_get_preserves_caller_alias_but_discharges_fresh_accumulator() {
    let model = AnalysisFixture::new(
        r#"
            type Item { value: string }

            function direct(items: Map<string, Item>, key: string) -> Item? {
              return items.get(key)
            }

            function local(key: string) -> Item? {
              final items = Map.empty<string, Item>()
              return items.get(key)
            }
        "#,
    )
    .module("responses")
    .package("agine.ai/llm-api")
    .analyze();

    assert_eq!(effects_in(&model, "responses", "direct"), no_effects());
    assert!(matches!(
        provenance_in(&model, "responses", "direct"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![caller_container_projection(0)]
    ));

    assert_eq!(
        effects_in(&model, "responses", "local"),
        no_effects(),
        "a fresh local Map must discharge its receiver alias and same-heap requirements"
    );
    assert!(matches!(
        provenance_in(&model, "responses", "local"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Fresh]
    ));
    assert!(model.resolved_call_targets().iter().any(|(_, target)| {
        matches!(
            target,
            ResolvedCallTarget::ReceiverBuiltin { op }
                if op.canonical_key == "receiver:Map.get@1"
        )
    }));
}

#[test]
fn exact_map_has_and_set_keep_contextual_receiver_semantics() {
    let model = AnalysisFixture::new(
        r#"
            type Item { value: string }

            function inspect(items: Map<string, Item>, key: string) -> bool {
              return items.has(key)
            }

            function updateCaller(items: Map<string, Item>, key: string, value: Item) -> void {
              return items.set(key, value)
            }

            function local(key: string, value: Item) -> bool {
              final items = Map.empty<string, Item>()
              items.set(key, value)
              return items.has(key)
            }
        "#,
    )
    .module("responses")
    .package("agine.ai/llm-api")
    .analyze();

    assert_eq!(effects_in(&model, "responses", "inspect"), no_effects());
    assert_eq!(effects_in(&model, "responses", "updateCaller"), no_effects());
    assert_eq!(
        effects_in(&model, "responses", "local"),
        no_effects(),
        "a fresh local Map must discharge set write and same-heap effects"
    );
    assert!(matches!(
        provenance_in(&model, "responses", "inspect"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Fresh]
    ));
    assert!(matches!(
        provenance_in(&model, "responses", "updateCaller"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::Constant]
    ));
    for canonical_key in ["receiver:Map.has@1", "receiver:Map.set@1"] {
        assert!(model.resolved_call_targets().iter().any(|(_, target)| {
            matches!(
                target,
                ResolvedCallTarget::ReceiverBuiltin { op }
                    if op.canonical_key == canonical_key
            )
        }));
    }
}

#[test]
fn formal_indexed_receiver_writes_ignore_unrelated_caller_actuals_through_helpers_and_scc() {
    let model = AnalysisFixture::new(
        r#"
            function add(headers: Array<string>, request: string) -> void {
              headers.push(request)
            }

            function nestedAdd(headers: Array<string>, request: string) -> void {
              add(headers, request)
            }

            function recursiveAdd(headers: Array<string>, request: string, again: bool) -> void {
              headers.push(request)
              if again { recursiveAdd(headers, request, false) }
            }

            function freshHeaders(request: string) -> void {
              final headers = Array.empty<string>()
              nestedAdd(headers, request)
              recursiveAdd(headers, request, true)
            }

            function callerHeaders(headers: Array<string>, request: string) -> void {
              nestedAdd(headers, request)
              recursiveAdd(headers, request, true)
            }
        "#,
    )
    .module("formal_write")
    .package("skiff.run/formal-write")
    .analyze();

    assert_eq!(
        effects_in(&model, "formal_write", "freshHeaders"),
        no_effects(),
        "a caller request actual must not make a Fresh headers receiver write caller-visible"
    );
    for callable in ["add", "nestedAdd", "recursiveAdd", "callerHeaders"] {
        assert_eq!(effects_in(&model, "formal_write", callable), no_effects(), "{callable}");
    }
}
