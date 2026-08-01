use super::support::*;

#[test]
fn post_construction_store_taints_fresh_return() {
    let model = AnalysisFixture::new(
        r#"
            type Child { value: string }
            type Holder { child: Child }

            function storeAndReturn(input: Child) -> Holder {
              const holder = Holder { child: Child { value: "fresh" } }
              holder.child = input
              return holder
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "storeAndReturn"),
        CallableMayEffects {
            returns_caller_alias: true,
            ..no_effects()
        }
    );
}

#[test]
fn post_construction_store_then_nested_mutation_fails_closed() {
    let model = AnalysisFixture::new(
        r#"
            type Child { value: string }
            type Holder { child: Child }

            function storeThenMutate(input: Child) -> Holder {
              const holder = Holder { child: Child { value: "fresh" } }
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
              const holder = Holder { child: Child { value: "fresh" } }
              const alias = holder
              alias.child = input
              return holder
            }
        "#,
    )
    .analyze();

    assert!(effects(&model, "aliasStore").returns_caller_alias);
    assert!(matches!(
        provenance(&model, "aliasStore"),
        CallableProvenanceSummary::Analyzed { .. }
    ));
}

#[test]
fn fresh_store_taint_propagates_through_callers_and_scc() {
    let model = AnalysisFixture::new(
        r#"
            type Child { value: string }
            type Holder { child: Child }

            function storeLeaf(input: Child) -> Holder {
              const holder = Holder { child: Child { value: "fresh" } }
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
        assert!(effects(&model, callable).returns_caller_alias, "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { .. }
        ));
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
              input.value = "changed"
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
        assert_eq!(
            effects(&model, callable),
            CallableMayEffects {
                writes_caller_reachable: true,
                ..no_effects()
            },
            "{callable}"
        );
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
              state.f01 = value
              state.f12 = "helper"
              state.f24 = value
            }

            function v1Proxy(events: Array<string>) -> string {
              const state = RelayState {
                f01: "", f02: "", f03: "", f04: "",
                f05: "", f06: "", f07: "", f08: "",
                f09: "", f10: "", f11: "", f12: "",
                f13: "", f14: "", f15: "", f16: "",
                f17: "", f18: "", f19: "", f20: "",
                f21: "", f22: "", f23: "", f24: ""
              }
              const alias = state
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

    assert_eq!(
        effects(&model, "update"),
        CallableMayEffects {
            writes_caller_reachable: true,
            ..no_effects()
        }
    );
    assert_eq!(effects(&model, "v1Proxy"), suspend_only_effects());
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
              input.child.value = "changed"
            }

            function reference(input: Holder, child: Child) -> void {
              input.child = child
            }

            function unknownRhs(input: Child, provider: any Provider) -> void {
              input.value = provider.value()
            }
        "#,
    )
    .analyze();

    assert_heap_store_fail_closed(&model, "nested");
    assert_eq!(
        effects(&model, "reference"),
        CallableMayEffects {
            writes_caller_reachable: true,
            ..no_effects()
        }
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
              const state = State { value: "" }
              state.value = "changed"
              const container = Map.empty<string, State>()
              container.set("state", state)
            }

            function intoArray() -> void {
              const state = State { value: "" }
              state.value = "changed"
              const container = Array.empty<State>()
              container.push(state)
            }

            function intoDatabase() -> void {
              const state = State { value: "" }
              state.value = "changed"
              db insert Stored { id = "state" state = state }
            }

            function ambiguousAlias(useSecond: bool) -> void {
              const first = State { value: "" }
              const second = State { value: "" }
              let alias = first
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
    assert_heap_store_fail_closed(&model, "intoDatabase");
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
              let state: State? = states.get(key)
              if state == null {
                state = State { value: "" }
              }
              state.value = "changed"
              return state
            }

            function local(key: string) -> State {
              const states = Map.empty<string, State>()
              let state: State? = states.get(key)
              if state == null {
                state = State { value: "" }
              }
              state.value = "changed"
              states.set(key, state)
              return state
            }

            function throughFresh(key: string) -> State {
              const states = Map.empty<string, State>()
              return formal(states, key)
            }

            type Node { child: Node? }

            function cycle() -> Node {
              const node = Node { child: null }
              node.child = node
              return node
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "formal"),
        CallableMayEffects {
            writes_caller_reachable: true,
            returns_caller_alias: true,
            ..no_effects()
        }
    );
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
              let state: State? = states.get(key)
              if state == null {
                state = State { key: key, value: "" }
                states.set(key, state)
              }
              return state
            }

            function local(key: string) -> State {
              const states = Map.empty<string, State>()
              const state = stateFor(states, key)
              state.value = "completed"
              states.set(key, state)
              return state
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "stateFor"),
        CallableMayEffects {
            writes_caller_reachable: true,
            returns_caller_alias: true,
            ..no_effects()
        }
    );
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
              const child = Child { parent: null }
              const parent = Parent { child: child }
              const selected = childOf(parent)
              selected.parent = parent
              return parent
            }

            type Node { next: Node? }

            function transitiveCycle() -> Node {
              const first = Node { next: null }
              const second = Node { next: null }
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
              const state = RelayState { bytes: 0 }
              const next = currentBytes(state) + 1
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
              const wrapper = wrap(input)
              const payload = wrapper.payload
              payload.set("kind", "changed")
            }

            function project(tools: Array<JsonObject>) -> void {
              const output = Array.empty<JsonObject>()
              for tool in tools {
                const projected: JsonObject = {
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
              let target: JsonObject = input
              if useFresh {
                const candidate: JsonObject = {
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
        CallableMayEffects {
            writes_caller_reachable: true,
            ..no_effects()
        },
        "mutating a caller payload recovered from a fresh wrapper is a write, not an identity observation"
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
        CallableMayEffects {
            writes_caller_reachable: true,
            ..no_effects()
        },
        "a fresh/caller receiver union must not discharge the caller candidate"
    );
}

#[test]
fn dependency_container_projection_can_be_mutated_and_reinserted_into_fresh_map() {
    let model = AnalysisFixture::new(
        r#"
            type State { key: string, value: string }

            function local(key: string) -> State {
              const states = Map.empty<string, State>()
              let state: State? = dep/tools/find(states, key)
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

    assert_eq!(effects(&model, "local"), suspend_only_effects());
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
              const wrapped: JsonObject = dep/tools/wrap(input)
              wrapped.set("kind", "function")
              return wrapped
            }

            function conditional(
              input: JsonObject,
              useFresh: bool
            ) -> void {
              let target: JsonObject = input
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
        CallableMayEffects {
            returns_caller_alias: true,
            may_suspend: true,
            ..no_effects()
        },
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
        CallableMayEffects {
            writes_caller_reachable: true,
            may_suspend: true,
            ..no_effects()
        },
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
              state.status = status
              state.snapshot = state.status
            }

            function local(status: string) -> StreamState {
              const state = StreamState {
                key: "response",
                status: "",
                snapshot: ""
              }
              update(state, status)
              return state
            }

            type Node { child: Node? }

            function selfStore(node: Node) -> void {
              node.child = node
            }

            function helperCycle() -> Node {
              const node = Node { child: null }
              selfStore(node)
              return node
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "update"),
        CallableMayEffects {
            writes_caller_reachable: true,
            ..no_effects()
        }
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

    assert!(effects(&model, "first").returns_caller_alias);
    assert!(effects(&model, "second").returns_caller_alias);
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
              const same = response == response
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
              const same = value == value
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
                const same = firstValue == firstValue
                return firstValue
              }
              const same = thirdValue == thirdValue
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
              const left: JsonObject = {}
              const right: JsonObject = {}
              return left == right
            }

            function identityThenFresh(input: JsonObject) -> JsonObject {
              const same = input == input
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
        assert!(effects.returns_caller_alias, "{callable}");
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
