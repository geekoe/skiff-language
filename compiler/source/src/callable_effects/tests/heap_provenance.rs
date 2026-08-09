use super::support::*;

#[test]
fn post_construction_store_taints_fresh_return() {
    let model = AnalysisFixture::new(
        r#"
            type Child { value: string }
            type Holder { child: Child }

            function storeAndReturn(input: Child) -> Holder {
              var holder = Holder { child: Child { value: "fresh" } }
              holder.child = input
              return holder
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "storeAndReturn"),
        no_effects(),
        "a fresh return embedding a caller payload is a logical snapshot"
    );
    assert!(matches!(
        provenance(&model, "storeAndReturn"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins.contains(&ValueProvenance::CallerParameter { index: 0 })
                && return_origins.contains(&ValueProvenance::Fresh)
    ));
}

#[test]
fn post_construction_store_then_nested_mutation_fails_closed() {
    let model = AnalysisFixture::new(
        r#"
            type Child { value: string }
            type Holder { child: Child }

            function storeThenMutate(input: Child) -> Holder {
              var holder = Holder { child: Child { value: "fresh" } }
              holder.child = input
              holder.child.value = "changed"
              return holder
            }
        "#,
    )
    .analyze();

    assert_heap_store_fail_closed(&model, "storeThenMutate");
}

#[test]
fn aliased_fresh_holder_store_taints_original_return() {
    let model = AnalysisFixture::new(
        r#"
            type Child { value: string }
            type Holder { child: Child }

            function aliasStore(input: Child) -> Holder {
              let holder = Holder { child: Child { value: "fresh" } }
              var alias = holder
              alias.child = input
              return holder
            }
        "#,
    )
    .analyze();

    assert_eq!(effects(&model, "aliasStore"), no_effects());
    assert!(matches!(
        provenance(&model, "aliasStore"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins.contains(&ValueProvenance::CallerParameter { index: 0 })
    ));
}

#[test]
fn fresh_store_taint_propagates_through_callers_and_scc() {
    let model = AnalysisFixture::new(
        r#"
            type Child { value: string }
            type Holder { child: Child }

            function storeLeaf(input: Child) -> Holder {
              var holder = Holder { child: Child { value: "fresh" } }
              holder.child = input
              return holder
            }

            function caller(input: Child) -> Holder {
              return storeLeaf(input)
            }

            function first(input: Child, stop: bool) -> Holder {
              if stop { return storeLeaf(input) }
              return second(input, true)
            }

            function second(input: Child, stop: bool) -> Holder {
              return first(input, stop)
            }
        "#,
    )
    .analyze();

    for callable in ["storeLeaf", "caller", "first", "second"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(
            matches!(
                provenance(&model, callable),
                CallableProvenanceSummary::Analyzed { return_origins, .. }
                    if return_origins.contains(&ValueProvenance::CallerParameter { index: 0 }),
            ),
            "{callable}"
        );
    }
}

#[test]
fn direct_parameter_field_store_has_write_without_identity_observation() {
    let model = AnalysisFixture::new(
        r#"
            type Boxed { value: string }

            impl Boxed {
              function clear() -> void {
                self.value = "cleared"
              }
            }

            function mutate(input: Boxed) -> void {
              var target = input
              target.value = "changed"
            }

            function wrapper(input: Boxed) -> void {
              mutate(input)
            }

            function methodWrapper(input: Boxed) -> void {
              input.clear()
            }
        "#,
    )
    .analyze();

    for callable in ["mutate", "wrapper", "Boxed.clear", "methodWrapper"] {
        // The write itself is tracked internally (write_parameters); the
        // retired writesCallerReachable aggregate flag no longer surfaces.
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
}

#[test]
fn fresh_alias_helper_loop_and_suspend_keep_relay_shaped_state_local() {
    let model = AnalysisFixture::new(
        r#"
            type RelayState {
              f01: string, f02: string, f03: string, f04: string,
              f05: string, f06: string, f07: string, f08: string,
              f09: string, f10: string, f11: string, f12: string,
              f13: string, f14: string, f15: string, f16: string,
              f17: string, f18: string, f19: string, f20: string,
              f21: string, f22: string, f23: string, f24: string
            }

            function update(state: RelayState, value: string) -> void {
              var writable = state
              writable.f01 = value
              writable.f12 = "helper"
              writable.f24 = value
            }

            function v1Proxy(events: Array<string>) -> string {
              var state = RelayState {
                f01: "", f02: "", f03: "", f04: "",
                f05: "", f06: "", f07: "", f08: "",
                f09: "", f10: "", f11: "", f12: "",
                f13: "", f14: "", f15: "", f16: "",
                f17: "", f18: "", f19: "", f20: "",
                f21: "", f22: "", f23: "", f24: ""
              }
              var alias = state
              alias.f02 = "local"
              for event in events {
                update(state, event)
                std.time.sleep(Duration.milliseconds(1))
                state.f23 = "after-suspend"
              }
              return state.f12
            }
        "#,
    )
    .analyze();

    assert_eq!(effects(&model, "update"), no_effects());
    assert_eq!(
        effects(&model, "v1Proxy"),
        pending_only_effects(vec![PendingEffectCategory::NativeCall]),
        "the sleep native carries the pending NativeCall category"
    );
    assert!(matches!(
        provenance(&model, "v1Proxy"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
}

#[test]
fn nested_heap_store_remains_fail_closed_and_direct_reference_store_is_precise() {
    let model = AnalysisFixture::new(
        r#"
            interface Provider {
              function value(self: Self) -> string
            }

            type Child { value: string }
            type Holder { child: Child }

            function nested(input: Holder) -> void {
              var target = input
              target.child.value = "changed"
            }

            function reference(input: Holder, child: Child) -> void {
              var target = input
              target.child = child
            }

            function unknownRhs(input: Child, provider: any Provider) -> void {
              var target = input
              target.value = provider.value()
            }
        "#,
    )
    .analyze();

    assert_heap_store_fail_closed(&model, "nested");
    assert_eq!(
        effects(&model, "reference"),
        no_effects(),
        "a direct reference store is an internal write, not a public effect"
    );
    assert_eq!(effects(&model, "unknownRhs"), all_effects());
    assert!(matches!(
        provenance(&model, "unknownRhs"),
        CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::UnknownCallTarget
        }
    ));
}

#[test]
fn mutated_fresh_root_can_enter_acyclic_local_containers_but_database_escape_fails_closed() {
    let model = AnalysisFixture::new(
        r#"
            type State { value: string }
            type Stored { id: string, state: State }

            db object Stored {
              primary key(id)
            }

            function intoMap() -> void {
              var state = State { value: "" }
              state.value = "changed"
              let container = Map.empty<string, State>()
              container.set("state", state)
            }

            function intoArray() -> void {
              var state = State { value: "" }
              state.value = "changed"
              let container = Array.empty<State>()
              container.push(state)
            }

            function intoDatabase() -> void {
              var state = State { value: "" }
              state.value = "changed"
              db insert Stored { id = "state" state = state }
            }

            function ambiguousAlias(useSecond: bool) -> void {
              let first = State { value: "" }
              let second = State { value: "" }
              var alias = first
              if useSecond {
                alias = second
              }
              alias.value = "ambiguous"
            }
        "#,
    )
    .analyze();

    for callable in ["intoMap", "intoArray", "ambiguousAlias"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
    assert_eq!(
        effects(&model, "intoDatabase"),
        CallableMayEffects {
            escapes_caller_value: true,
            requires_same_heap_identity: false,
            invokes_unknown_target: true,
            may_pending: true,
            pending_effect_categories: vec![
                PendingEffectCategory::Unknown,
                PendingEffectCategory::HostEffect
            ],
            inout_path_effects: Vec::new(),
        }
    );
    assert_eq!(
        provenance(&model, "intoDatabase"),
        &CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::UnsupportedHeapStore,
        }
    );
}

#[test]
fn conditional_map_lookup_tracks_distinct_fresh_and_formal_candidates() {
    let model = AnalysisFixture::new(
        r#"
            type State { value: string }

            function formal(
              states: Map<string, State>,
              key: string
            ) -> State {
              var state: State? = states.get(key)
              if state == null {
                state = State { value: "" }
              }
              state.value = "changed"
              return state
            }

            function local(key: string) -> State {
              let states = Map.empty<string, State>()
              var state: State? = states.get(key)
              if state == null {
                state = State { value: "" }
              }
              state.value = "changed"
              states.set(key, state)
              return state
            }

            function throughFresh(key: string) -> State {
              let states = Map.empty<string, State>()
              return formal(states, key)
            }

            type Node { child: Node? }

            function cycle() -> Node {
              var node = Node { child: null }
              node.child = node
              return node
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "formal"),
        no_effects(),
        "mutating a caller map projection is tracked internally only"
    );
    assert!(matches!(
        provenance(&model, "formal"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins.contains(&ValueProvenance::Fresh)
                && return_origins.iter().any(|origin| matches!(
                    origin,
                    ValueProvenance::CallerParameter { .. }
                        | ValueProvenance::CallerParameterProjection { .. }
                ))
    ));
    for callable in ["local", "throughFresh"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
    }
    assert_heap_store_fail_closed(&model, "cycle");
}

#[test]
fn helper_map_projection_can_be_mutated_and_reinserted_without_becoming_the_map_root() {
    let model = AnalysisFixture::new(
        r#"
            type State { key: string, value: string }

            function stateFor(
              states: Map<string, State>,
              key: string
            ) -> State {
              var state: State? = states.get(key)
              if state == null {
                state = State { key: key, value: "" }
                states.set(key, state)
              }
              return state
            }

            function local(key: string) -> State {
              let states = Map.empty<string, State>()
              var state = stateFor(states, key)
              state.value = "completed"
              states.set(key, state)
              return state
            }
        "#,
    )
    .analyze();

    assert_eq!(effects(&model, "stateFor"), no_effects());
    assert!(matches!(
        provenance(&model, "stateFor"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins.iter().any(|origin| matches!(
                origin,
                ValueProvenance::CallerParameter { .. }
                    | ValueProvenance::CallerParameterProjection { .. }
            ))
    ));
    assert_eq!(effects(&model, "local"), no_effects());
    assert!(matches!(
        provenance(&model, "local"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
}

#[test]
fn helper_field_projection_keeps_parent_edge_and_rejects_real_cycle() {
    let model = AnalysisFixture::new(
        r#"
            type Child { parent: Parent? }
            type Parent { child: Child }

            function childOf(parent: Parent) -> Child {
              return parent.child
            }

            function cycle() -> Parent {
              let child = Child { parent: null }
              let parent = Parent { child: child }
              var selected = childOf(parent)
              selected.parent = parent
              return parent
            }

            type Node { next: Node? }

            function transitiveCycle() -> Node {
              var first = Node { next: null }
              var second = Node { next: null }
              first.next = second
              second.next = first
              return first
            }
        "#,
    )
    .analyze();

    assert_eq!(
        provenance(&model, "childOf"),
        &CallableProvenanceSummary::Analyzed {
            return_origins: vec![caller_field_projection(0, "child")],
            direct_return_origins: vec![caller_field_projection(0, "child")],
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        }
    );
    assert_heap_store_fail_closed(&model, "cycle");
    assert_heap_store_fail_closed(&model, "transitiveCycle");
}

#[test]
fn scalar_field_projection_does_not_invent_a_heap_cycle_in_relay_state_updates() {
    let model = AnalysisFixture::new(
        r#"
            type RelayState { bytes: integer }

            function currentBytes(state: RelayState) -> integer {
              return state.bytes
            }

            function local() -> RelayState {
              var state = RelayState { bytes: 0 }
              let next = currentBytes(state) + 1
              state.bytes = next
              return state
            }
        "#,
    )
    .analyze();

    assert_eq!(effects(&model, "currentBytes"), no_effects());
    assert_eq!(effects(&model, "local"), no_effects());
    assert!(matches!(
        provenance(&model, "local"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
}

#[test]
fn fresh_json_root_stays_distinct_from_caller_reachable_payload() {
    let model = AnalysisFixture::new(
        r#"
            type Wrapper { payload: JsonObject }

            function wrap(input: JsonObject) -> Wrapper {
              return Wrapper { payload: input }
            }

            function mutateWrappedPayload(input: JsonObject) -> void {
              let wrapper = wrap(input)
              let payload = wrapper.payload
              payload.set("kind", "changed")
            }

            function project(tools: Array<JsonObject>) -> void {
              let output = Array.empty<JsonObject>()
              for tool in tools {
                let projected: JsonObject = {
                  payload: tool.get("payload")
                }
                projected.set("kind", "function")
                output.push(projected)
              }
            }

            function conditional(
              input: JsonObject,
              useFresh: bool
            ) -> void {
              var target: JsonObject = input
              if useFresh {
                let candidate: JsonObject = {
                  payload: input.get("payload")
                }
                target = candidate
              }
              target.set("kind", "function")
            }
        "#,
    )
    .analyze();

    assert_eq!(
        provenance(&model, "wrap"),
        &CallableProvenanceSummary::Analyzed {
            return_origins: vec![
                ValueProvenance::Fresh,
                ValueProvenance::CallerParameter { index: 0 },
            ],
            direct_return_origins: vec![ValueProvenance::Fresh],
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        },
        "the fresh wrapper is direct while its caller-owned payload remains reachable"
    );
    assert_eq!(
        effects(&model, "mutateWrappedPayload"),
        no_effects(),
        "mutating a caller payload recovered from a fresh wrapper is tracked internally, not an identity observation"
    );

    assert_eq!(
        effects(&model, "project"),
        no_effects(),
        "mutating the fresh projected object and fresh output array must remain local"
    );
    assert!(matches!(
        provenance(&model, "project"),
        CallableProvenanceSummary::Analyzed { .. }
    ));

    assert_eq!(
        effects(&model, "conditional"),
        no_effects(),
        "a fresh/caller receiver union must not discharge the caller candidate"
    );
}

#[test]
fn dependency_container_projection_can_be_mutated_and_reinserted_into_fresh_map() {
    let model = AnalysisFixture::new(
        r#"
            type State { key: string, value: string }

            function local(key: string) -> State {
              let states = Map.empty<string, State>()
              var state: State? = dep/tools/find(states, key)
              if state == null {
                state = State { key: key, value: "" }
                states.set(key, state)
              }
              state.value = "completed"
              states.set(key, state)
              return state
            }
        "#,
    )
    .dependency_analysis(container_projection_dependency())
    .analyze();

    assert_eq!(
        effects(&model, "local"),
        pending_only_effects(vec![PendingEffectCategory::Unknown]),
        "the dependency call override carries the exact-signature suspension channel"
    );
    assert!(matches!(
        provenance(&model, "local"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
}

#[test]
fn dependency_fresh_wrapper_keeps_payload_reachable_without_becoming_caller_owned() {
    let model = AnalysisFixture::new(
        r#"
            function mutate(
              input: JsonObject
            ) -> JsonObject {
              let wrapped: JsonObject = dep/tools/wrap(input)
              wrapped.set("kind", "function")
              return wrapped
            }

            function conditional(
              input: JsonObject,
              useFresh: bool
            ) -> void {
              var target: JsonObject = input
              if useFresh {
                target = dep/tools/wrap(input)
              }
              target.set("kind", "function")
            }
        "#,
    )
    .dependency_analysis(fresh_wrapper_dependency())
    .analyze();

    assert_eq!(
        effects(&model, "mutate"),
        pending_only_effects(vec![PendingEffectCategory::Unknown]),
        "mutating the dependency's fresh return root must not become a caller write"
    );
    assert_eq!(
        provenance(&model, "mutate"),
        &CallableProvenanceSummary::Analyzed {
            return_origins: vec![
                ValueProvenance::Fresh,
                ValueProvenance::Constant,
                ValueProvenance::CallerParameter { index: 0 },
                ValueProvenance::DependencyReturn {
                    callable_id: "pkg-callable:dep-tools-wrap".into(),
                },
            ],
            direct_return_origins: vec![ValueProvenance::Fresh],
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        }
    );
    assert_eq!(
        effects(&model, "conditional"),
        pending_only_effects(vec![PendingEffectCategory::Unknown]),
        "a direct Fresh/caller union remains conservative across a package boundary"
    );
}

#[test]
fn helper_parameter_store_distinguishes_field_projection_from_root_cycle() {
    let model = AnalysisFixture::new(
        r#"
            type StreamState {
              key: string,
              status: string,
              snapshot: string
            }

            function update(state: StreamState, status: string) -> void {
              var writable = state
              writable.status = status
              writable.snapshot = writable.status
            }

            function local(status: string) -> StreamState {
              let state = StreamState {
                key: "response",
                status: "",
                snapshot: ""
              }
              update(state, status)
              return state
            }

            type Node { child: Node? }

            function selfStore(node: Node) -> void {
              var writable = node
              writable.child = node
            }

            function helperCycle() -> Node {
              let node = Node { child: null }
              selfStore(node)
              return node
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "update"),
        no_effects(),
        "direct parameter field stores are internal writes"
    );
    assert_eq!(effects(&model, "local"), no_effects());
    assert!(matches!(
        provenance(&model, "local"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
    assert_heap_store_fail_closed(&model, "helperCycle");
}

#[test]
fn recursive_scc_reaches_alias_fixed_point() {
    let model = AnalysisFixture::new(
        r#"
            type Boxed { value: string }

            function first(input: Boxed, stop: bool) -> Boxed {
              if stop { return input }
              return second(input, true)
            }

            function second(input: Boxed, stop: bool) -> Boxed {
              return first(input, stop)
            }
        "#,
    )
    .analyze();

    for callable in ["first", "second"] {
        assert!(
            matches!(
                provenance(&model, callable),
                CallableProvenanceSummary::Analyzed { return_origins, .. }
                    if return_origins.contains(&ValueProvenance::CallerParameter { index: 0 }),
            ),
            "{callable}"
        );
    }
}

#[test]
fn recursively_growing_projection_path_fails_closed_at_the_wire_limit() {
    let model = AnalysisFixture::new(
        r#"
            type Node { child: Node }

            function descend(node: Node, stop: bool) -> Node {
              if stop { return node.child }
              return descend(node.child, true)
            }
        "#,
    )
    .analyze();

    assert_eq!(effects(&model, "descend"), all_effects());
    assert!(matches!(
        provenance(&model, "descend"),
        CallableProvenanceSummary::Unknown { .. }
    ));
}

#[test]
fn local_call_transfer_maps_alias_and_identity_to_exact_formal_actuals() {
    let model = AnalysisFixture::new(
        r#"
            function withRequestCors(
              request: JsonObject,
              settings: JsonObject,
              response: JsonObject
            ) -> JsonObject {
              let same = response == response
              return response
            }

            function thirdHop(
              first: JsonObject,
              second: JsonObject,
              value: JsonObject
            ) -> JsonObject {
              return withRequestCors(first, second, value)
            }

            function freshThird(input: JsonObject) -> JsonObject {
              return thirdHop(input, {}, {})
            }

            function first(
              value: JsonObject,
              second: JsonObject,
              third: JsonObject
            ) -> JsonObject {
              let same = value == value
              return value
            }

            function callerFirst(input: JsonObject) -> JsonObject {
              return first(input, {}, {})
            }

            function branch(
              chooseFirst: bool,
              firstValue: JsonObject,
              thirdValue: JsonObject
            ) -> JsonObject {
              if chooseFirst {
                let same = firstValue == firstValue
                return firstValue
              }
              let same = thirdValue == thirdValue
              return thirdValue
            }

            function eitherFormal(
              chooseFirst: bool,
              input: JsonObject
            ) -> JsonObject {
              return branch(chooseFirst, input, {})
            }

            function callerInequality(input: JsonObject) -> bool {
              return input != input
            }

            function recursiveIdentity(
              input: JsonObject,
              again: bool
            ) -> bool {
              if again {
                return recursiveIdentity(input, false)
              }
              return input == input
            }

            function callerRecursive(input: JsonObject) -> bool {
              return recursiveIdentity(input, true)
            }

            function freshRecursive() -> bool {
              return recursiveIdentity({}, true)
            }

            function freshEquality() -> bool {
              let left: JsonObject = {}
              let right: JsonObject = {}
              return left == right
            }

            function identityThenFresh(input: JsonObject) -> JsonObject {
              let same = input == input
              return {}
            }
        "#,
    )
    .analyze();

    assert_eq!(effects(&model, "freshThird"), no_effects());
    let CallableProvenanceSummary::Analyzed { return_origins, .. } =
        provenance(&model, "freshThird")
    else {
        panic!("fresh third actual must retain analyzed provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh]);

    for (callable, expected_parameter) in [("callerFirst", 0), ("eitherFormal", 1)] {
        let effects = effects(&model, callable);
        assert!(effects.requires_same_heap_identity, "{callable}");
        let CallableProvenanceSummary::Analyzed { return_origins, .. } =
            provenance(&model, callable)
        else {
            panic!("{callable} must retain analyzed provenance");
        };
        assert!(
            return_origins.contains(&ValueProvenance::CallerParameter {
                index: expected_parameter
            }),
            "{callable}: {return_origins:?}"
        );
    }

    for callable in [
        "callerInequality",
        "recursiveIdentity",
        "callerRecursive",
        "identityThenFresh",
    ] {
        assert_eq!(
            effects(&model, callable),
            CallableMayEffects {
                requires_same_heap_identity: true,
                ..no_effects()
            },
            "{callable}"
        );
    }
    for callable in ["freshRecursive", "freshEquality"] {
        assert_eq!(effects(&model, callable), no_effects(), "{callable}");
    }
}
