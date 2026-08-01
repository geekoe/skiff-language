use super::support::*;

#[test]
fn normal_return_and_wire_detached_throw_remain_independent() {
    let model = analyze(
        r#"
            type Boxed { value: string }

            function returnAlias(input: Boxed) -> Boxed {
              return input
            }

            function throwAlias(input: Boxed) -> void {
              throw input
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    let returned = effects(&model, "returnAlias");
    assert!(returned.returns_caller_alias);
    assert!(!returned.throws_caller_alias);
    let thrown = effects(&model, "throwAlias");
    assert!(!thrown.returns_caller_alias);
    assert!(!thrown.throws_caller_alias);
    assert!(matches!(
        provenance(&model, "throwAlias"),
        CallableProvenanceSummary::Analyzed { throw_origins, .. }
            if throw_origins == &vec![ValueProvenance::Fresh]
    ));
}

#[test]
fn throw_and_rethrow_preserve_operand_effects_but_detach_emitted_provenance() {
    let model = analyze(
        r#"
            type Boxed { value: string }
            type Failure { message: string }

            function buildFailure(input: Boxed) -> Failure {
              input.value = "changed"
              std.time.sleep(Duration.milliseconds(1))
              return Failure { message: input.value }
            }

            function throwStatement(input: Boxed) -> void {
              throw buildFailure(input)
            }

            function throwExpression(input: Boxed) -> Failure {
              return throw buildFailure(input)
            }

            function rethrowStatement(input: Boxed) -> void {
              const attempted = catch<Failure>(throw Failure { message: input.value })
              if attempted.tag == "err" {
                rethrow attempted.exception
              }
            }

            function rethrowExpression(input: Boxed) -> Failure {
              const attempted = catch<Failure>(throw Failure { message: input.value })
              if attempted.tag == "err" {
                return rethrow attempted.exception
              }
              return Failure { message: "unreachable" }
            }

            function nestedRethrow(input: Boxed) -> void {
              const outer = catch<Failure>(rethrowStatement(input))
              if outer.tag == "err" {
                rethrow outer.exception
              }
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    for callable in [
        "throwStatement",
        "throwExpression",
        "rethrowStatement",
        "rethrowExpression",
        "nestedRethrow",
    ] {
        let effects = effects(&model, callable);
        assert!(!effects.throws_caller_alias, "{callable}: {effects:?}");
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

    assert!(effects(&model, "throwStatement").writes_caller_reachable);
    assert!(effects(&model, "throwExpression").writes_caller_reachable);
    assert!(effects(&model, "throwStatement").may_suspend);
    assert!(effects(&model, "throwExpression").may_suspend);
    assert!(!effects(&model, "rethrowStatement").writes_caller_reachable);
    assert!(!effects(&model, "rethrowExpression").writes_caller_reachable);
}

#[test]
fn stream_spawn_database_and_callback_escape_lanes_are_explicit() {
    let model = analyze(
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
              spawn sink(input)
            }

            function persist(input: Boxed) -> void {
              db insert Stored { id = input.id payload = input }
            }

            function callback(input: Boxed) -> void {
              const boxed = input as Provider
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    assert_escape_lane(&model, "stream", ValueEscapeLane::Stream);
    assert_escape_lane(&model, "scalarStream", ValueEscapeLane::Stream);
    assert_escape_lane(&model, "spawnWork", ValueEscapeLane::Spawn);
    assert!(effects(&model, "persist").may_suspend);
    assert_escape_lane(&model, "persist", ValueEscapeLane::Database);
    assert_escape_lane(&model, "callback", ValueEscapeLane::Callback);
    assert!(
        !effects(&model, "callback").requires_same_heap_identity,
        "interface boxing is a callback escape, not an identity observation"
    );
}

#[test]
fn database_queries_and_detached_writes_do_not_escape_caller_values() {
    let model = analyze(
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
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["read", "history", "put", "compareAndSet"] {
        assert_eq!(
            effects(&model, callable),
            suspend_only_effects(),
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
    let model = analyze(
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
        SourceDependencyAnalysisInput::default(),
    );

    for callable in ["insertOwned", "replaceOwned"] {
        let callable_effects = effects(&model, callable);
        assert!(callable_effects.may_suspend, "{callable}");
        assert!(callable_effects.escapes_caller_value, "{callable}");
        assert_escape_lane(&model, callable, ValueEscapeLane::Database);
    }
}

#[test]
fn database_value_transactions_transfer_the_exact_final_value() {
    let model = analyze(
        r#"
            type Pointer { target: string }
            type Input { pointer: Pointer }
            type Receipt { sequence: integer, pointer: Pointer }

            function receipt(input: Input) -> Receipt {
              return db transaction value {
                const pointer = input.pointer
                Receipt { sequence: 1, pointer: pointer }
              }
            }

            function direct(input: Input) -> Input {
              return db transaction value {
                input
              }
            }
        "#,
        SourceDependencyAnalysisInput::default(),
    );

    let receipt = effects(&model, "receipt");
    assert!(receipt.may_suspend);
    assert!(receipt.returns_caller_alias);
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

    let direct = effects(&model, "direct");
    assert!(direct.may_suspend);
    assert!(direct.returns_caller_alias);
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
    let model = analyze(
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
              const result = db upsert Stored(input.id) {
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
        SourceDependencyAnalysisInput::default(),
    );

    assert_eq!(effects(&model, "projected"), suspend_only_effects());
    let direct = effects(&model, "direct");
    assert!(direct.may_suspend);
    assert!(direct.escapes_caller_value);
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
    let model = analyze_named(
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
              const stream = std.bytes.fromUtf8("fresh")
              nestedForward(stream, state)
              recursiveForward(stream, state, true)
            }

            function callerStream(stream: bytes, state: JsonObject) -> void {
              nestedForward(stream, state)
              recursiveForward(stream, state, true)
            }
        "#,
        SourceDependencyAnalysisInput::default(),
        "formal_escape",
        "skiff.run/formal-escape",
    );

    assert_eq!(
        effects_in(&model, "formal_escape", "freshStream"),
        CallableMayEffects {
            may_suspend: true,
            ..no_effects()
        },
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
                may_suspend: true,
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
