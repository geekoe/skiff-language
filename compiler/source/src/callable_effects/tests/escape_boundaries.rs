use super::support::*;

#[test]
fn normal_return_and_wire_detached_throw_remain_independent() {
    let model = AnalysisFixture::new(
        r#"
            type Boxed { value: string }

            function returnAlias(input: Boxed) -> Boxed {
              return input
            }

            function throwAlias(input: Boxed) -> void {
              throw input
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "returnAlias"),
        no_effects(),
        "returning a caller aggregate is a logical snapshot, not an alias effect"
    );
    assert!(matches!(
        provenance(&model, "returnAlias"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![ValueProvenance::CallerParameter { index: 0 }]
    ));
    assert_eq!(
        effects(&model, "throwAlias"),
        no_effects(),
        "throwing a caller aggregate detaches at the wire boundary"
    );
    assert!(matches!(
        provenance(&model, "throwAlias"),
        CallableProvenanceSummary::Analyzed { throw_origins, .. }
            if throw_origins == &vec![ValueProvenance::Fresh]
    ));
}

#[test]
fn throw_and_rethrow_preserve_operand_effects_but_detach_emitted_provenance() {
    let model = AnalysisFixture::new(
        r#"
            type Boxed { value: string }
            type Failure { message: string }

            function buildFailure(input: Boxed) -> Failure {
              var target = input
              target.value = "changed"
              std.time.sleep(Duration.milliseconds(1))
              return Failure { message: target.value }
            }

            function throwStatement(input: Boxed) -> void {
              throw buildFailure(input)
            }

            function throwExpression(input: Boxed) -> Failure {
              return throw buildFailure(input)
            }

            function rethrowStatement(input: Boxed) -> void {
              final attempted = catch<Failure>(throw Failure { message: input.value })
              if attempted.tag == "err" {
                rethrow attempted.exception
              }
            }

            function rethrowExpression(input: Boxed) -> Failure {
              final attempted = catch<Failure>(throw Failure { message: input.value })
              if attempted.tag == "err" {
                return rethrow attempted.exception
              }
              return Failure { message: "unreachable" }
            }

            function nestedRethrow(input: Boxed) -> void {
              final outer = catch<Failure>(rethrowStatement(input))
              if outer.tag == "err" {
                rethrow outer.exception
              }
            }
        "#,
    )
    .analyze();

    for callable in [
        "throwStatement",
        "throwExpression",
        "rethrowStatement",
        "rethrowExpression",
        "nestedRethrow",
    ] {
        let effects = effects(&model, callable);
        assert!(
            !effects.requires_same_heap_identity,
            "{callable}: {effects:?}"
        );
        assert!(!effects.invokes_unknown_target, "{callable}: {effects:?}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Analyzed { throw_origins, .. }
                if throw_origins == &vec![ValueProvenance::Fresh]
        ));
    }

    // Operand mutation is tracked internally (write_parameters) and no longer
    // surfaces as a public aggregate write flag; only the operand's pending
    // NativeCall (std.time.sleep) propagates to the throwing callables.
    assert_eq!(
        effects(&model, "throwStatement"),
        pending_only_effects(vec![PendingEffectCategory::NativeCall])
    );
    assert_eq!(
        effects(&model, "throwExpression"),
        pending_only_effects(vec![PendingEffectCategory::NativeCall])
    );
    assert_eq!(effects(&model, "rethrowStatement"), no_effects());
    assert_eq!(effects(&model, "rethrowExpression"), no_effects());
}

#[test]
fn stream_task_database_and_callback_escape_lanes_are_explicit() {
    let model = AnalysisFixture::new(
        r#"
            interface Provider {
              function name(self: Self) -> string
            }

            type Boxed implements Provider { id: string, value: string }
            type Stored { id: string, payload: Boxed }
            impl Boxed {
              function name() -> string { return self.value }
            }

            db object Stored {
              primary key(id)
            }

            function sink(input: Boxed) -> void {}

            function stream(input: Boxed) -> Stream<Boxed> {
              emit(input)
            }

            function scalarStream(input: string) -> Stream<string> {
              emit(input)
            }

            function spawnWork(input: Boxed) -> void {
              dispatch sink(input)
            }

            function expressionSpawn(input: Boxed) -> void {
              final ref = dispatch sink(input)
            }

            function persist(input: Boxed) -> void {
              db insert Stored { id = input.id payload = input }
            }

            function callback(input: Boxed) -> void {
              final boxed = input as Provider
            }
        "#,
    )
    .analyze();

    assert_escape_lane(&model, "stream", ValueEscapeLane::Stream);
    assert_escape_lane(&model, "scalarStream", ValueEscapeLane::Stream);
    assert_escape_lane(&model, "spawnWork", ValueEscapeLane::Dispatch);
    assert_escape_lane(&model, "expressionSpawn", ValueEscapeLane::Dispatch);
    assert_eq!(
        effects(&model, "persist"),
        CallableMayEffects {
            escapes_caller_value: true,
            may_pending: true,
            pending_effect_categories: vec![PendingEffectCategory::HostEffect],
            ..no_effects()
        }
    );
    assert_escape_lane(&model, "persist", ValueEscapeLane::Database);
    assert_escape_lane(&model, "callback", ValueEscapeLane::Callback);
    assert!(
        !effects(&model, "callback").requires_same_heap_identity,
        "interface boxing is a callback escape, not an identity observation"
    );
}

#[test]
fn stream_for_in_consumers_record_stream_pending_effects() {
    let model = AnalysisFixture::new(
        r#"
            function consume(values: Stream<number>) -> number {
              final stream = values
              for item in stream {}
              return 0
            }

            function consumeArray(values: Array<number>) -> number {
              for item in values {}
              return 0
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "consume"),
        pending_only_effects(vec![PendingEffectCategory::Stream])
    );
    assert_eq!(effects(&model, "consumeArray"), no_effects());
}

#[test]
fn database_queries_and_detached_writes_do_not_escape_caller_values() {
    let model = AnalysisFixture::new(
        r#"
            type Payload { value: string }
            type Stored { id: string, payload: Payload }

            db object Stored {
              primary key(id)
            }

            function read(id: string) -> Stored? {
              return db optional Stored(id)
            }

            function history(id: string) -> Array<Stored> {
              return db find many Stored { where id == id }
            }

            function put(id: string, value: string) -> Stored {
              return db insert Stored {
                id = id
                payload = Payload { value: value }
              }
            }

            function compareAndSet(id: string, value: string) -> Stored? {
              return db update Stored(id) {
                payload = Payload { value: value }
              }
            }
        "#,
    )
    .analyze();

    for callable in ["read", "history", "put", "compareAndSet"] {
        assert_eq!(
            effects(&model, callable),
            pending_only_effects(vec![PendingEffectCategory::HostEffect]),
            "{callable}"
        );
        let CallableProvenanceSummary::Analyzed {
            return_origins,
            escape_lanes,
            ..
        } = provenance(&model, callable)
        else {
            panic!("{callable} must retain exact database provenance");
        };
        assert_eq!(return_origins, &vec![ValueProvenance::Fresh], "{callable}");
        assert!(escape_lanes.is_empty(), "{callable}: {escape_lanes:?}");
    }
}

#[test]
fn persisting_caller_owned_mutable_values_remains_a_database_escape() {
    let model = AnalysisFixture::new(
        r#"
            type Payload { value: string }
            type Stored { id: string, payload: Payload }

            db object Stored {
              primary key(id)
            }

            function insertOwned(id: string, payload: Payload) -> Stored {
              return db insert Stored { id = id payload = payload }
            }

            function replaceOwned(id: string, payload: Payload) -> Stored? {
              return db update Stored(id) { payload = payload }
            }
        "#,
    )
    .analyze();

    for callable in ["insertOwned", "replaceOwned"] {
        let callable_effects = effects(&model, callable);
        assert_eq!(
            callable_effects,
            CallableMayEffects {
                escapes_caller_value: true,
                may_pending: true,
                pending_effect_categories: vec![PendingEffectCategory::HostEffect],
                ..no_effects()
            },
            "{callable}"
        );
        assert_escape_lane(&model, callable, ValueEscapeLane::Database);
    }
}

#[test]
fn database_value_transactions_transfer_the_exact_final_value() {
    let model = AnalysisFixture::new(
        r#"
            type Pointer { target: string }
            type Input { pointer: Pointer }
            type Receipt { sequence: integer, pointer: Pointer }

            function receipt(input: Input) -> Receipt {
              return db transaction value {
                final pointer = input.pointer
                Receipt { sequence: 1, pointer: pointer }
              }
            }

            function direct(input: Input) -> Input {
              return db transaction value {
                input
              }
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "receipt"),
        pending_only_effects(vec![PendingEffectCategory::HostEffect])
    );
    let CallableProvenanceSummary::Analyzed { return_origins, .. } = provenance(&model, "receipt")
    else {
        panic!("transaction result must retain analyzed provenance");
    };
    assert_eq!(
        return_origins,
        &vec![
            ValueProvenance::Fresh,
            ValueProvenance::Constant,
            caller_field_projection(0, "pointer")
        ]
    );

    assert_eq!(
        effects(&model, "direct"),
        pending_only_effects(vec![PendingEffectCategory::HostEffect])
    );
    let CallableProvenanceSummary::Analyzed { return_origins, .. } = provenance(&model, "direct")
    else {
        panic!("direct caller result should retain exact caller provenance");
    };
    assert_eq!(
        return_origins,
        &vec![ValueProvenance::CallerParameter { index: 0 }]
    );
}

#[test]
fn database_writes_detach_static_field_projections_but_not_direct_or_unknown_values() {
    let model = AnalysisFixture::new(
        r#"
            interface Provider {
              function value(self: Self) -> string
            }

            type Pointer { target: string }
            type Input { id: string, pointer: Pointer }
            type Stored { id: string, pointer: Pointer }

            db object Stored {
              primary key(id)
            }

            function projected(input: Input) -> Stored {
              final result = db upsert Stored(input.id) {
                id = input.id
                pointer = input.pointer
              } {
                pointer = input.pointer
              }
              return result.value
            }

            function direct(input: Input, pointer: Pointer) -> Stored {
              return db insert Stored {
                id = input.id
                pointer = pointer
              }
            }

            function unknownPredicate(input: Input, provider: any Provider) -> void {
              db update many Stored {
                where id == provider.value()
              } {
                pointer = input.pointer
              }
              return null
            }

            function unknownUpdate(input: Input, provider: any Provider) -> Stored? {
              return db update Stored(input.id) {
                id = provider.value()
              }
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "projected"),
        pending_only_effects(vec![PendingEffectCategory::HostEffect])
    );
    assert_eq!(
        effects(&model, "direct"),
        CallableMayEffects {
            escapes_caller_value: true,
            may_pending: true,
            pending_effect_categories: vec![PendingEffectCategory::HostEffect],
            ..no_effects()
        }
    );
    assert_escape_lane(&model, "direct", ValueEscapeLane::Database);

    for callable in ["unknownPredicate", "unknownUpdate"] {
        let callable_effects = effects(&model, callable);
        assert!(callable_effects.invokes_unknown_target, "{callable}");
        assert!(matches!(
            provenance(&model, callable),
            CallableProvenanceSummary::Unknown {
                reason: CallableProvenanceUnknownReason::UnknownCallTarget
            }
        ));
    }
}

#[test]
fn formal_indexed_stream_escape_ignores_unrelated_caller_actuals_through_helpers_and_scc() {
    let model = AnalysisFixture::new(
        r#"
            function forward(stream: bytes, state: JsonObject) -> void {
              emit(stream)
            }

            function nestedForward(stream: bytes, state: JsonObject) -> void {
              forward(stream, state)
            }

            function recursiveForward(stream: bytes, state: JsonObject, again: bool) -> void {
              emit(stream)
              if again { recursiveForward(stream, state, false) }
            }

            function freshStream(state: JsonObject) -> void {
              final stream = std.bytes.fromUtf8("fresh")
              nestedForward(stream, state)
              recursiveForward(stream, state, true)
            }

            function callerStream(stream: bytes, state: JsonObject) -> void {
              nestedForward(stream, state)
              recursiveForward(stream, state, true)
            }
        "#,
    )
    .module("formal_escape")
    .package("skiff.run/formal-escape")
    .analyze();

    assert_eq!(
        effects_in(&model, "formal_escape", "freshStream"),
        pending_only_effects(vec![PendingEffectCategory::Stream]),
        "a caller state actual must not enter the Stream lane selected by the Fresh stream"
    );
    for callable in [
        "forward",
        "nestedForward",
        "recursiveForward",
        "callerStream",
    ] {
        assert_eq!(
            effects_in(&model, "formal_escape", callable),
            CallableMayEffects {
                escapes_caller_value: true,
                may_pending: true,
                pending_effect_categories: vec![PendingEffectCategory::Stream],
                ..no_effects()
            },
            "{callable}"
        );
        assert!(matches!(
            provenance_in(&model, "formal_escape", callable),
            CallableProvenanceSummary::Analyzed { escape_lanes, .. }
                if escape_lanes == &vec![ValueEscapeLane::Stream]
        ));
    }
}
