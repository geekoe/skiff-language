use std::fs;

use serde_json::Value;
use skiff_compiler::test_support::compile_source_file_ir_artifact_for_test as compile_source_file_ir_artifact;

mod common;
use common::{
    artifacts::{build_temp_service_publication, source_artifact},
    TestDir,
};

#[test]
fn function_ir_emits_typed_slot_layout_and_refs() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            function run(input: number) -> number {
                const total = input
                if true {
                    const total = 2
                    const copied = total
                }
                return total
            }
        "#,
        "internal/slots.skiff",
        "internal.slots",
        "service",
    )
    .expect("slot fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");

    let input_slot = slot_index(run, "input", "param");
    let total_slots = slot_indexes(run, "total", "local");
    assert_eq!(
        total_slots.len(),
        2,
        "outer and inner total should be separate local slots"
    );
    assert_ne!(total_slots[0], total_slots[1]);
    let copied_slot = slot_index(run, "copied", "local");

    let outer_total = let_stmt_by_slot(run, total_slots[0]);
    assert_eq!(
        load_slot(expr_for_ref(run, &outer_total["value"])),
        input_slot
    );

    let copied = let_stmt_by_slot(run, copied_slot);
    assert_eq!(
        load_slot(expr_for_ref(run, &copied["value"])),
        total_slots[1]
    );

    let return_stmt = find_stmt(run, |stmt| stmt["kind"] == "return")
        .expect("return statement should be present");
    assert_eq!(
        load_slot(expr_for_ref(run, &return_stmt["value"])),
        total_slots[0],
        "return after nested block should resolve to outer total"
    );
}

#[test]
fn same_scope_duplicate_let_is_rejected_before_ir_emission() {
    let error = compile_source_file_ir_artifact(
        r#"
            function run() -> number {
                const value = 1
                const value = 2
                return value
            }
        "#,
        "internal/duplicate.skiff",
        "internal.duplicate",
        "service",
    )
    .expect_err("same-scope duplicate let should be a compiler error")
    .to_string();
    let lower = error.to_ascii_lowercase();

    assert!(
        lower.contains("value")
            && (lower.contains("duplicate")
                || lower.contains("already declared")
                || lower.contains("redeclared")),
        "duplicate binding error should name the duplicate variable, got:\n{error}"
    );
}

#[test]
fn public_single_file_helper_lowers_interface_box_with_expression_type_facts() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            interface Provider {
              function name(self: Self) -> string
            }

            type Host implements Provider {}

            impl Host {
              function name() -> string {
                return "host"
              }
            }

            function run() -> string {
              const provider = Host{} as Provider
              return provider.name()
            }
        "#,
        "internal/interface_box_helper.skiff",
        "internal.interface_box_helper",
        "service",
    )
    .expect("single-file helper should lower interface boxing with expression facts");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");
    let interface_boxes = run["body"]["expressions"]
        .as_array()
        .expect("expressions should be an array")
        .iter()
        .filter(|expr| expr["kind"] == "interfaceBox")
        .collect::<Vec<_>>();

    assert_eq!(
        interface_boxes.len(),
        1,
        "Host{{}} as Provider should lower to exactly one interfaceBox"
    );
    assert_eq!(
        interface_boxes[0]["source"]["kind"], "local",
        "single-file helper should produce a local interface method table"
    );
}

#[test]
fn static_callees_are_typed_call_targets_while_receiver_roots_are_slots() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            function addOne(value: number) -> number {
                return value + 1
            }

            function run() -> number {
                const result = addOne(1)
                const second = internal.callees.addOne(result)
                const list: Array<number> = Array.empty<number>()
                list.push(second)
                return list.length()
            }
        "#,
        "internal/callees.skiff",
        "internal.callees",
        "service",
    )
    .expect("callee fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");

    let calls = call_exprs(run);
    let local_calls = calls
        .iter()
        .filter(|call| call["target"]["kind"] == "localExecutable")
        .count();
    assert!(
        local_calls >= 2,
        "unqualified and same-module-qualified local calls should lower to typed localExecutable targets"
    );
    assert!(
        has_native_call(&calls, "Array", "empty", "core.array.empty"),
        "Array.empty should lower to a shared native target with a stable binding key"
    );

    let list_slot = slot_index(run, "list", "local");
    let push = receiver_builtin_call(run, "Array", "push").expect("list.push receiver call");
    assert_eq!(
        load_slot(expr_for_ref(run, &push["args"][0])),
        list_slot,
        "receiver object for list.push should be a loadSlot"
    );
}

#[test]
fn shared_native_alias_callees_win_over_builtin_roots() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            function run() -> number {
                const items: Array<string> = Array.empty<string>()
                items.push("ok")
                const joined = string.join(items, ",")
                const parsed = number.parse("1")
                const body: bytes = bytes.fromUtf8(joined)
                return body.length()
            }
        "#,
        "internal/native_aliases.skiff",
        "internal.native_aliases",
        "service",
    )
    .expect("native alias fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");
    let calls = call_exprs(run);

    assert!(has_native_call(
        &calls,
        "std.string",
        "join",
        "std.string.join"
    ));
    assert!(has_native_call(
        &calls,
        "std.number",
        "parse",
        "core.number.parse"
    ));
    assert!(has_native_call(
        &calls,
        "std.bytes",
        "fromUtf8",
        "core.bytes.fromUtf8"
    ));
}

#[test]
fn std_http_json_infers_native_type_arg_from_record_payload() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            import std

            type JsonOutput {
              marker: string,
              count: number,
            }

            function plain() -> std.http.HttpResponse {
                return std.http.json(200, JsonOutput {
                  marker: "ok",
                  count: 1
                })
            }

            function withHeaders() -> std.http.HttpResponse {
                const headers = Array.empty<std.http.HttpHeader>()
                return std.http.jsonWithHeaders(200, JsonOutput {
                  marker: "ok",
                  count: 2
                }, headers)
            }
        "#,
        "internal/http_json_type_args.skiff",
        "internal.http_json_type_args",
        "service",
    )
    .expect("std.http JSON response helpers should infer native type args from record payloads");
    let artifact_value = artifact.value();
    let plain = executable_entry(&artifact_value, "plain");
    let with_headers = executable_entry(&artifact_value, "withHeaders");

    assert_eq!(
        native_call(plain, "std.http", "json")["typeArgs"]["T0"]["kind"],
        "localType",
        "std.http.json should carry direct native typeArgs.T0 for the payload record"
    );
    assert_eq!(
        native_call(with_headers, "std.http", "jsonWithHeaders")["typeArgs"]["T0"]["kind"],
        "localType",
        "std.http.jsonWithHeaders should carry direct native typeArgs.T0 for the payload record"
    );
}

#[test]
fn receiver_mutation_and_assignment_lower_to_typed_targets() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            type Session {
              players: Array<string>,
              title: string
            }

            function run(session: Session, memberId: string) -> number {
                session.players.push(memberId)
                session.title = "updated"
                return session.players.length()
            }
        "#,
        "internal/mutable_paths.skiff",
        "internal.mutable_paths",
        "service",
    )
    .expect("mutable path fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");
    let session_slot = slot_index(run, "session", "param");

    let push = receiver_builtin_call(run, "Array", "push").expect("push call");
    let receiver = expr_for_ref(run, &push["args"][0]);
    assert_eq!(receiver["kind"], "field");
    assert_eq!(receiver["field"], "players");
    assert_eq!(
        load_slot(expr_for_ref(run, &receiver["object"])),
        session_slot
    );

    let assignment = find_stmt(run, |stmt| stmt["kind"] == "assign")
        .expect("session.title assignment should lower");
    assert_eq!(assignment["target"]["kind"], "field");
    assert_eq!(assignment["target"]["field"], "title");
    assert_eq!(
        load_slot(expr_for_ref(run, &assignment["target"]["object"])),
        session_slot
    );
}

#[test]
fn user_impl_receiver_call_lowers_to_static_executable() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            type User {
              first: string,
              last: string
            }

            impl User {
              function displayName() -> string {
                return self.first
              }
            }

            function run(user: User) -> string {
                return user.displayName()
            }
        "#,
        "internal/user_receiver.skiff",
        "internal.user_receiver",
        "service",
    )
    .expect("user impl receiver fixture should compile");
    let artifact_value = artifact.value();
    let method_index = executable_index(&artifact_value, "User.displayName");
    let run = executable_entry(&artifact_value, "run");

    let call = call_exprs(run)
        .into_iter()
        .find(|call| {
            call["target"]["kind"] == "localExecutable"
                && call["target"]["executableIndex"].as_u64() == Some(method_index)
        })
        .expect("user.displayName should lower to localExecutable");
    assert_eq!(call["args"].as_array().expect("call args").len(), 1);
    assert!(
        dynamic_receiver_call(run, "displayName").is_none(),
        "ordinary user impl receiver call must not lower to DynamicReceiver"
    );
}

#[test]
fn generic_impl_receiver_call_lowers_to_static_executable() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            type Box<T> {
              value: T
            }

            impl Box<T> {
              function unwrap() -> T {
                return self.value
              }
            }

            function run(box: Box<string>) -> string {
                return box.unwrap()
            }
        "#,
        "internal/generic_receiver.skiff",
        "internal.generic_receiver",
        "service",
    )
    .expect("generic impl receiver fixture should compile");
    let artifact_value = artifact.value();
    let method_index = executable_index(&artifact_value, "Box<T>.unwrap");
    let run = executable_entry(&artifact_value, "run");

    assert!(
        call_exprs(run).into_iter().any(|call| {
            call["target"]["kind"] == "localExecutable"
                && call["target"]["executableIndex"].as_u64() == Some(method_index)
        }),
        "generic impl receiver call should lower to the impl method executable"
    );
    assert!(
        dynamic_receiver_call(run, "unwrap").is_none(),
        "generic impl receiver call must not lower to DynamicReceiver"
    );
}

#[test]
fn ordinary_erased_object_receiver_call_is_rejected_before_dynamic_receiver() {
    let error = compile_source_file_ir_artifact(
        r#"
            function run(item: {}) -> number {
                return item.length()
            }
        "#,
        "internal/erased_receiver.skiff",
        "internal.erased_receiver",
        "service",
    )
    .expect_err("ordinary erased object receiver should not lower dynamically")
    .to_string();

    assert!(
        error.contains("must resolve to a local/package executable"),
        "unexpected erased receiver error: {error}"
    );
}

#[test]
fn json_object_receiver_call_lowers_to_receiver_builtin() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            function run(item: JsonObject) -> Json {
                return item.get("name")
            }
        "#,
        "internal/json_object_receiver.skiff",
        "internal.json_object_receiver",
        "service",
    )
    .expect("JsonObject receiver fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");

    assert_eq!(
        artifact_value["requiredReceiverBuiltinCapabilityVersion"], 1,
        "receiver builtin calls should record required capability version"
    );
    assert!(
        receiver_builtin_call(run, "JsonObject", "get").is_some(),
        "JsonObject.get should lower to receiverBuiltin"
    );
}

#[test]
fn chained_string_receiver_call_lowers_to_receiver_builtin() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            type SessionConfig {
              cookieName: string
            }

            function run(settings: SessionConfig, token: string) -> string {
                let value = settings.cookieName.concat("=")
                return value.concat(token)
            }
        "#,
        "internal/chained_string_receiver.skiff",
        "internal.chained_string_receiver",
        "package",
    )
    .expect("chained string receiver fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");

    assert!(
        receiver_builtin_call(run, "string", "concat").is_some(),
        "string.concat should lower to receiverBuiltin"
    );
}

#[test]
fn publication_string_receiver_facts_flow_through_config_and_db_body() {
    let temp = TestDir::new("skiff-runtime-slots", "db-body-string-receiver");
    fs::create_dir_all(temp.path().join("internal")).unwrap();
    fs::write(
        temp.path().join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("internal").join("db_receiver.skiff"),
        r#"
            import std

            type RuntimeLiveDoc {
              id: string,
              value: string,
              visits: number,
              rank: number,
            }

            db object RuntimeLiveDoc {
              name "runtime_live_doc"
              primary key(id)
            }

            function run() -> bool {
                const marker = config.require<string>("runtimeLive.db")
                const prefix = "runtime-live-db-".concat(std.crypto.uuidSimple())
                const firstId = prefix.concat("-a")
                db insert RuntimeLiveDoc { id = firstId value = marker.concat("-first") visits = 1 rank = 10 }
                return firstId.contains(marker)
            }
        "#,
    )
    .unwrap();

    let published = build_temp_service_publication(temp.path());
    let artifact = source_artifact(&published, "internal/db_receiver.skiff");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");
    let concat_calls = call_exprs(run)
        .into_iter()
        .filter(|call| receiver_builtin_call_matches(call, "string", "concat"))
        .count();

    assert!(
        concat_calls >= 3,
        "config string, chained string, and db body string.concat calls should lower as receiverBuiltin calls: {run}"
    );
    assert!(
        receiver_builtin_call(run, "string", "contains").is_some(),
        "string.contains should keep the static receiver fact through publication lowering"
    );
}

#[test]
fn array_empty_binding_receiver_call_lowers_to_receiver_builtin() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            type Event {
              id: string
            }

            function run(event: Event) -> Array<Event> {
                const events = Array.empty<Event>()
                events.push(event)
                return events
            }
        "#,
        "internal/array_empty_receiver.skiff",
        "internal.array_empty_receiver",
        "package",
    )
    .expect("Array.empty receiver fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");

    assert!(
        receiver_builtin_call(run, "Array", "push").is_some(),
        "Array.push should lower to receiverBuiltin"
    );
}

#[test]
fn literal_string_binding_receiver_call_lowers_to_receiver_builtin() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            function run(delta: string) -> string {
                let activeText = ""
                activeText = activeText.concat(delta)
                return activeText
            }
        "#,
        "internal/literal_string_receiver.skiff",
        "internal.literal_string_receiver",
        "package",
    )
    .expect("literal string receiver fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");

    assert!(
        receiver_builtin_call(run, "string", "concat").is_some(),
        "literal string concat should lower to receiverBuiltin"
    );
}

#[test]
fn string_replace_all_receiver_call_lowers_to_receiver_builtin() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            function run(value: string) -> string {
                return value.replaceAll("-", "_")
            }
        "#,
        "internal/string_replace_all_receiver.skiff",
        "internal.string_replace_all_receiver",
        "package",
    )
    .expect("string replaceAll receiver fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");

    assert!(
        receiver_builtin_call(run, "string", "replaceAll").is_some(),
        "string.replaceAll should lower to receiverBuiltin"
    );
}

#[test]
fn stream_item_bytes_to_string_contains_receiver_chain_lowers_to_receiver_builtin() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            type Chunk {
              value: bytes
            }

            function run(chunks: Stream<Chunk>) -> bool {
                for chunk in chunks {
                    const text = chunk.value.toUtf8String()
                    if text.contains("data:") {
                        return true
                    }
                }
                return false
            }
        "#,
        "internal/stream_contains_receiver.skiff",
        "internal.stream_contains_receiver",
        "service",
    )
    .expect("stream item bytes-to-string contains receiver fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");

    assert!(
        receiver_builtin_call(run, "bytes", "toUtf8String").is_some(),
        "bytes.toUtf8String should lower to receiverBuiltin"
    );
    assert!(
        receiver_builtin_call(run, "string", "contains").is_some(),
        "string.contains should lower to receiverBuiltin"
    );
}

#[test]
fn std_http_body_bytes_receiver_chain_lowers_to_receiver_builtin() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            import std

            function requestText(request: std.http.HttpRequest) -> string {
                return request.body.toUtf8String()
            }

            function responseText(response: std.http.HttpResponse) -> string {
                return response.body.toUtf8String()
            }
        "#,
        "internal/http_body_receiver.skiff",
        "internal.http_body_receiver",
        "service",
    )
    .expect("std.http body bytes receiver fixture should compile");
    let artifact_value = artifact.value();
    let request_text = executable_entry(&artifact_value, "requestText");
    let response_text = executable_entry(&artifact_value, "responseText");

    assert!(
        receiver_builtin_call(request_text, "bytes", "toUtf8String").is_some(),
        "HttpRequest.body bytes.toUtf8String should lower to receiverBuiltin"
    );
    assert!(
        receiver_builtin_call(response_text, "bytes", "toUtf8String").is_some(),
        "HttpResponse.body bytes.toUtf8String should lower to receiverBuiltin"
    );
}

#[test]
fn actor_ref_receiver_call_is_rejected() {
    let error = compile_source_file_ir_artifact(
        r#"
            type ThreadActor {
              id: string
            }

            function run(actor: ActorRef<ThreadActor>) -> void {
                actor.receive("ping")
                return
            }
        "#,
        "internal/actor_receiver.skiff",
        "internal.actor_receiver",
        "service",
    )
    .expect_err("ActorRef receiver calls should be rejected")
    .to_string();

    assert!(
        error.contains("ActorRef receiver method calls are no longer supported"),
        "unexpected ActorRef receiver error: {error}"
    );
}

#[test]
fn nominal_match_pattern_is_rejected_before_runtime_type_lookup() {
    let error = compile_source_file_ir_artifact(
        r#"
            type User {
              status: string
            }

            function run(user: User) -> string {
                match user {
                  User { status } => {
                    return status
                  }
                  _ => {
                    return "unknown"
                  }
                }
            }
        "#,
        "internal/nominal_pattern.skiff",
        "internal.nominal_pattern",
        "service",
    )
    .expect_err("nominal match pattern should be rejected before File IR emits PatternIr::Type")
    .to_string();

    assert!(
        error.contains("nominal pattern `User` cannot match an erased runtime value"),
        "unexpected nominal pattern error: {error}"
    );
}

#[test]
fn record_literal_and_binding_patterns_do_not_emit_type_pattern_ir() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            function run(user: { status: string }) -> string {
                match user {
                  { status } => {
                    return status
                  }
                  "active" => {
                    return "literal"
                  }
                  other => {
                    return "other"
                  }
                }
            }
        "#,
        "internal/structural_pattern.skiff",
        "internal.structural_pattern",
        "service",
    )
    .expect("structural/literal/binding patterns should compile without PatternIr::Type");

    assert!(
        !json_contains_pattern_type(&artifact.value()),
        "ordinary structural/literal/binding patterns must not emit PatternIr::Type: {}",
        artifact.value()
    );
}

#[test]
fn object_db_single_write_results_are_not_read_record_wrappers() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            type User {
              id: string,
              name: string,
              visits: number
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run(rows: Array<User>) -> bool {
                const inserted = db insert User { id = "u1" name = "Ada" visits = 0 }
                const updated = db update User("u1") { visits += 1 }
                const replaced = db replace User("u1") { name = "Grace" visits = 2 }
                const upserted = db upsert User("u1") { name = "Ada" visits = 0 } { visits += 1 }
                const insertedMany = db insert many User values rows
                const updatedMany = db update many User { where name != null } { visits += 1 }
                const deletedMany = db delete many User { where name == "Ada" }
                return true
            }
        "#,
        "internal/db_write_results.skiff",
        "internal.db_write_results",
        "service",
    )
    .expect("object db write result fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");
    let operations = db_operations(run);

    let insert = db_operation(&operations, "insert", false);
    assert_user_db_object_symbol(&insert["resultType"]);

    let update = db_operation(&operations, "update", false);
    assert_eq!(update["resultType"]["kind"], "nullable");
    assert_user_db_object_symbol(&update["resultType"]["inner"]);

    let replace = db_operation(&operations, "replace", false);
    assert_eq!(replace["resultType"]["kind"], "nullable");
    assert_user_db_object_symbol(&replace["resultType"]["inner"]);

    let upsert = db_operation(&operations, "upsert", false);
    assert_eq!(upsert["resultType"]["kind"], "builtin");
    assert_eq!(upsert["resultType"]["name"], "DbUpsertResult");
    assert_user_db_object_symbol(&upsert["resultType"]["args"][0]);

    assert_builtin_type(
        &db_operation(&operations, "insert", true)["resultType"],
        "DbInsertManyResult",
    );
    assert_builtin_type(
        &db_operation(&operations, "update", true)["resultType"],
        "DbUpdateManyResult",
    );
    assert_builtin_type(
        &db_operation(&operations, "delete", true)["resultType"],
        "DbDeleteManyResult",
    );
}

#[test]
fn object_db_upsert_result_fields_lower_to_static_field_access() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            type User {
              id: string,
              name: string,
              visits: number
            }

            db object User {
              name "user"
              primary key(id)
            }

            function run() -> bool {
                const result = db upsert User("u1") { name = "Ada" visits = 0 } { visits += 1 }
                const inserted = result.inserted
                const name = result.value.name
                const visits = result.value.visits
                if inserted {
                    return name == "Ada"
                }
                return visits == 1
            }
        "#,
        "internal/db_upsert_result_fields.skiff",
        "internal.db_upsert_result_fields",
        "service",
    )
    .expect("object db upsert result field fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");

    assert!(
        count_field_exprs(run, "inserted") >= 1,
        "DbUpsertResult.inserted should lower as a static field expression: {run}"
    );
    assert!(
        count_field_exprs(run, "value") >= 2,
        "DbUpsertResult.value should lower as static field expressions: {run}"
    );
    assert!(
        count_field_exprs(run, "name") >= 1,
        "DbUpsertResult.value.name should lower as a static field expression: {run}"
    );
    assert!(
        count_field_exprs(run, "visits") >= 1,
        "DbUpsertResult.value.visits should lower as a static field expression: {run}"
    );
    for field in ["inserted", "value", "name", "visits"] {
        assert!(
            dynamic_receiver_call(run, field).is_none(),
            "db upsert result field `{field}` must not lower to DynamicReceiver"
        );
    }
}

#[test]
fn object_db_single_write_results_are_readonly() {
    for (name, source, expected) in [
        (
            "insert",
            r#"
                type User {
                  id: string,
                  name: string,
                  visits: number
                }

                db object User {
                  name "user"
                  primary key(id)
                }

                function run() -> bool {
                    const user = db insert User { id = "u1" name = "Ada" visits = 0 }
                    user.name = "Grace"
                    return true
                }
            "#,
            "cannot assign to field of readonly binding `user`",
        ),
        (
            "update",
            r#"
                type User {
                  id: string,
                  name: string,
                  visits: number
                }

                db object User {
                  name "user"
                  primary key(id)
                }

                function run() -> bool {
                    const user = db update User("u1") { visits += 1 }
                    user.name = "Grace"
                    return true
                }
            "#,
            "unknown field `name` on User?",
        ),
        (
            "replace",
            r#"
                type User {
                  id: string,
                  name: string,
                  visits: number
                }

                db object User {
                  name "user"
                  primary key(id)
                }

                function run() -> bool {
                    const user = db replace User("u1") { name = "Grace" visits = 2 }
                    user.name = "Ada"
                    return true
                }
            "#,
            "unknown field `name` on User?",
        ),
        (
            "upsert",
            r#"
                type User {
                  id: string,
                  name: string,
                  visits: number
                }

                db object User {
                  name "user"
                  primary key(id)
                }

                function run() -> bool {
                    const result = db upsert User("u1") { name = "Ada" visits = 0 } { visits += 1 }
                    result.value.name = "Grace"
                    return true
                }
            "#,
            "cannot assign to field of readonly binding `result`",
        ),
    ] {
        let error = compile_source_file_ir_artifact(
            source,
            format!("internal/db_write_readonly_{name}.skiff"),
            format!("internal.db_write_readonly_{name}"),
            "service",
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains(expected),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn map_keys_and_for_in_lower_to_typed_slots() {
    let artifact = compile_source_file_ir_artifact(
        r#"
            type UserId = string
            type User { name: string }

            function keepUserId(id: UserId) -> UserId {
                return id
            }

            function keepUser(user: User) -> string {
                return user.name
            }

            function run(users: Map<UserId, User>) -> Array<UserId> {
                const ids: Array<UserId> = users.keys()
                for id in users {
                    const copy: UserId = keepUserId(id)
                }
                for id, user in users {
                    const copy: UserId = keepUserId(id)
                    const name: string = keepUser(user)
                }
                return ids
            }
        "#,
        "internal/map_for.skiff",
        "internal.map_for",
        "service",
    )
    .expect("map keys and for-in fixture should compile");
    let artifact_value = artifact.value();
    let run = executable_entry(&artifact_value, "run");
    let for_in = for_in_stmts(run);
    assert_eq!(for_in.len(), 2, "expected single and entry map for-in");

    assert!(
        for_in[0].get("valueSlot").is_none(),
        "single-binding map for should not carry valueSlot: {}",
        for_in[0]
    );
    assert_eq!(
        slot_name_by_index(run, for_in[0]["itemSlot"].as_u64().unwrap()),
        Some("id"),
        "single-binding map for should bind the key slot"
    );

    assert!(
        for_in[1].get("valueSlot").is_some(),
        "entry map for should carry valueSlot: {}",
        for_in[1]
    );
    assert_eq!(
        slot_name_by_index(run, for_in[1]["itemSlot"].as_u64().unwrap()),
        Some("id"),
        "entry map for itemSlot is the key slot"
    );
    assert_eq!(
        slot_name_by_index(run, for_in[1]["valueSlot"].as_u64().unwrap()),
        Some("user"),
        "entry map for valueSlot is the value slot"
    );
    assert!(
        receiver_builtin_call(run, "Map", "keys").is_some(),
        "users.keys() should lower as receiverBuiltin"
    );
}

#[test]
fn entry_for_rejects_array_and_stream_iterables() {
    for (name, source, expected) in [
        (
            "array",
            r#"
                function run(values: Array<string>) -> number {
                    for key, value in values {
                    }
                    return 1
                }
            "#,
            "for entry binding requires Map",
        ),
        (
            "stream",
            r#"
                function run(values: Stream<string>) -> number {
                    for key, value in values {
                    }
                    return 1
                }
            "#,
            "for entry binding requires Map",
        ),
    ] {
        let error = compile_source_file_ir_artifact(
            source,
            format!("internal/entry_for_{name}.skiff"),
            format!("internal.entry_for_{name}"),
            "service",
        )
        .expect_err("non-map entry for should fail")
        .to_string();
        assert!(
            error.contains(expected),
            "unexpected {name} entry-for error: {error}"
        );
    }
}

#[test]
fn map_for_bindings_are_immutable_and_non_duplicate() {
    for (name, source, expected) in [
        (
            "single_assignment",
            r#"
                type UserId = string
                type User { name: string }
                function run(users: Map<UserId, User>) -> number {
                    for key in users {
                        key = UserId("x")
                    }
                    return 1
                }
            "#,
            "cannot assign to immutable binding `key`",
        ),
        (
            "entry_assignment",
            r#"
                type UserId = string
                type User { name: string }
                function run(users: Map<UserId, User>, other: User) -> number {
                    for key, value in users {
                        value = other
                    }
                    return 1
                }
            "#,
            "cannot assign to immutable binding `value`",
        ),
        (
            "duplicate_entry",
            r#"
                type UserId = string
                type User { name: string }
                function run(users: Map<UserId, User>) -> number {
                    for key, key in users {
                    }
                    return 1
                }
            "#,
            "duplicate binding `key`",
        ),
    ] {
        let error = compile_source_file_ir_artifact(
            source,
            format!("internal/map_for_{name}.skiff"),
            format!("internal.map_for_{name}"),
            "service",
        )
        .expect_err("invalid map for binding should fail")
        .to_string();
        assert!(
            error.contains(expected),
            "unexpected map for binding error for {name}: {error}"
        );
    }
}

fn executable_entry<'a>(artifact: &'a Value, name: &str) -> &'a Value {
    artifact["executables"]
        .as_array()
        .expect("executables should be an array")
        .iter()
        .find(|executable| {
            executable["symbol"]
                .as_str()
                .is_some_and(|symbol| symbol.ends_with(&format!(".{name}")))
        })
        .unwrap_or_else(|| panic!("executable {name} should be present"))
}

fn executable_index(artifact: &Value, name: &str) -> u64 {
    artifact["executables"]
        .as_array()
        .expect("executables should be an array")
        .iter()
        .position(|executable| {
            executable["symbol"]
                .as_str()
                .is_some_and(|symbol| symbol.ends_with(&format!(".{name}")))
        })
        .unwrap_or_else(|| panic!("executable {name} should be present")) as u64
}

fn slot_index(executable: &Value, name: &str, kind: &str) -> u64 {
    let slots = slot_indexes(executable, name, kind);
    assert_eq!(
        slots.len(),
        1,
        "expected exactly one {kind} slot for {name}, got {slots:?}"
    );
    slots[0]
}

fn slot_indexes(executable: &Value, name: &str, kind: &str) -> Vec<u64> {
    executable["slots"]["slots"]
        .as_array()
        .expect("slots.slots should be an array")
        .iter()
        .filter(|slot| slot["name"] == name && slot["kind"] == kind)
        .map(|slot| slot["index"].as_u64().expect("slot index"))
        .collect()
}

fn slot_name_by_index(executable: &Value, index: u64) -> Option<&str> {
    executable["slots"]["slots"]
        .as_array()
        .expect("slots.slots should be an array")
        .iter()
        .find(|slot| slot["index"].as_u64() == Some(index))
        .and_then(|slot| slot["name"].as_str())
}

fn let_stmt_by_slot(executable: &Value, slot: u64) -> &Value {
    find_stmt(executable, |stmt| {
        stmt["kind"] == "let" && stmt["slot"].as_u64() == Some(slot)
    })
    .unwrap_or_else(|| panic!("let statement for slot {slot} should be present"))
}

fn find_stmt(executable: &Value, predicate: impl Fn(&Value) -> bool) -> Option<&Value> {
    executable["body"]["statements"]
        .as_array()?
        .iter()
        .find(|stmt| predicate(stmt))
}

fn for_in_stmts(executable: &Value) -> Vec<&Value> {
    executable["body"]["statements"]
        .as_array()
        .expect("body.statements should be an array")
        .iter()
        .filter(|stmt| stmt["kind"] == "forIn")
        .collect()
}

fn call_exprs(executable: &Value) -> Vec<&Value> {
    executable["body"]["expressions"]
        .as_array()
        .expect("expressions should be an array")
        .iter()
        .filter_map(|expr| {
            if expr["kind"] == "call" {
                Some(&expr["call"])
            } else {
                None
            }
        })
        .collect()
}

fn dynamic_receiver_call<'a>(executable: &'a Value, method_name: &str) -> Option<&'a Value> {
    call_exprs(executable).into_iter().find(|call| {
        call["target"]["kind"] == "dynamicReceiver" && call["target"]["methodName"] == method_name
    })
}

fn receiver_builtin_call<'a>(
    executable: &'a Value,
    receiver: &str,
    method_name: &str,
) -> Option<&'a Value> {
    call_exprs(executable)
        .into_iter()
        .find(|call| receiver_builtin_call_matches(call, receiver, method_name))
}

fn receiver_builtin_call_matches(call: &Value, receiver: &str, method_name: &str) -> bool {
    call["target"]["kind"] == "receiverBuiltin"
        && call["target"]["op"]["receiver"] == receiver
        && call["target"]["op"]["method"] == method_name
}

fn json_contains_pattern_type(value: &Value) -> bool {
    if value.get("kind").and_then(Value::as_str) == Some("type") && value.get("ty").is_some() {
        return true;
    }
    match value {
        Value::Array(items) => items.iter().any(json_contains_pattern_type),
        Value::Object(object) => object.values().any(json_contains_pattern_type),
        _ => false,
    }
}

fn count_field_exprs(value: &Value, field: &str) -> usize {
    let current = usize::from(
        value.get("kind").and_then(Value::as_str) == Some("field")
            && value.get("field").and_then(Value::as_str) == Some(field),
    );
    current
        + match value {
            Value::Array(items) => items
                .iter()
                .map(|item| count_field_exprs(item, field))
                .sum(),
            Value::Object(object) => object
                .values()
                .map(|item| count_field_exprs(item, field))
                .sum(),
            _ => 0,
        }
}

fn has_native_call(
    calls: &[&Value],
    namespace: &str,
    symbol_name: &str,
    binding_key: &str,
) -> bool {
    calls.iter().any(|call| {
        call["target"]["kind"] == "native"
            && call["target"]["target"]["namespace"] == namespace
            && call["target"]["target"]["symbol"] == symbol_name
            && call["target"]["target"]["bindingKey"] == binding_key
    })
}

fn native_call<'a>(executable: &'a Value, namespace: &str, symbol_name: &str) -> &'a Value {
    call_exprs(executable)
        .into_iter()
        .find(|call| {
            call["target"]["kind"] == "native"
                && call["target"]["target"]["namespace"] == namespace
                && call["target"]["target"]["symbol"] == symbol_name
        })
        .unwrap_or_else(|| panic!("native call {namespace}.{symbol_name} should be present"))
}

fn expr_for_ref<'a>(executable: &'a Value, expr_ref: &Value) -> &'a Value {
    let index = expr_ref["expression"]
        .as_u64()
        .expect("expression ref should contain expression index") as usize;
    &executable["body"]["expressions"][index]
}

fn load_slot(expr: &Value) -> u64 {
    assert_eq!(
        expr["kind"], "loadSlot",
        "expected loadSlot expression: {expr}"
    );
    expr["slot"].as_u64().expect("loadSlot.slot")
}

fn db_operations(executable: &Value) -> Vec<&Value> {
    executable["body"]["expressions"]
        .as_array()
        .expect("expressions should be an array")
        .iter()
        .filter_map(|expr| {
            if expr["kind"] == "dbOperation" {
                Some(&expr["operation"])
            } else {
                None
            }
        })
        .collect()
}

fn db_operation<'a>(operations: &'a [&Value], op: &str, many: bool) -> &'a Value {
    operations
        .iter()
        .copied()
        .find(|operation| operation["op"] == op && operation["many"] == many)
        .unwrap_or_else(|| panic!("db {op} many={many} operation should be present"))
}

fn assert_user_db_object_symbol(ty: &Value) {
    assert_eq!(ty["kind"], "dbObjectSymbol");
    assert_eq!(
        ty["symbol"],
        serde_json::json!({ "modulePath": "internal.db_write_results", "symbol": "User" })
    );
    assert!(
        !serde_json::to_string(ty).unwrap().contains("readRecord"),
        "{ty}"
    );
}

fn assert_builtin_type(ty: &Value, name: &str) {
    assert_eq!(ty["kind"], "builtin");
    assert_eq!(ty["name"], name);
}
