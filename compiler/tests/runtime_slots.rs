use std::fs;

use serde_json::Value;
use skiff_compiler_emission::PublishedFileIrArtifact;

mod common;
use common::{
    artifacts::module_artifact,
    package_project::{compile_package_project, PackageProjectCompileError},
    TestDir,
};

fn compile_package_file_ir(
    source: &str,
    source_path: impl AsRef<str>,
    module_path: impl AsRef<str>,
) -> Result<PublishedFileIrArtifact, PackageProjectCompileError> {
    let temp = TestDir::new("skiff-compiler", "runtime-slots-package");
    let package_manifest = "id: example.com/runtime-slots\nversion: 1.0.0\n";
    fs::write(temp.path().join("package.yml"), package_manifest)
        .expect("package manifest should be written");
    fs::write(temp.path().join("api.yml"), "{}\n").expect("api.yml should be written");
    let source_file = temp.path().join(source_path.as_ref());
    fs::create_dir_all(
        source_file
            .parent()
            .expect("fixture source should have a parent directory"),
    )
    .expect("fixture source directory should be created");
    fs::write(source_file, source).expect("fixture source should be written");

    let project = compile_package_project(temp.path())?;
    Ok(module_artifact(&project.package, module_path.as_ref()).clone())
}

fn compile_root_alias_array_push(
    overlay_source: &str,
) -> Result<common::package_project::PublishedPackageProject, PackageProjectCompileError> {
    let temp = TestDir::new("skiff-compiler", "union-array-push");
    fs::write(
        temp.path().join("package.yml"),
        "id: example.com/union-array-push\nversion: 1.0.0\n",
    )
    .expect("package manifest should be written");
    fs::write(temp.path().join("api.yml"), "{}\n").expect("api.yml should be written");
    fs::write(
        temp.path().join("types.skiff"),
        r#"alias Modality = "text" | "image" | "video" | "audio""#,
    )
    .expect("production type source should be written");
    fs::write(temp.path().join("types_overlay.skiff"), overlay_source)
        .expect("overlay source should be written");
    compile_package_project(temp.path())
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

fn init_slot_stmt_by_slot(executable: &Value, slot: u64) -> &Value {
    find_stmt(executable, |stmt| {
        stmt["kind"] == "initSlot" && stmt["slot"].as_u64() == Some(slot)
    })
    .unwrap_or_else(|| panic!("init slot statement for slot {slot} should be present"))
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

fn json_contains_applied_nominal_pattern(value: &Value) -> bool {
    if value.get("kind").and_then(Value::as_str) == Some("type") {
        let ty = &value["ty"];
        if ty["kind"] == "appliedNominal"
            && ty["base"]["kind"] == "localType"
            && ty["arguments"][0]["kind"] == "builtin"
            && ty["arguments"][0]["name"] == "string"
        {
            return true;
        }
    }
    match value {
        Value::Array(items) => items.iter().any(json_contains_applied_nominal_pattern),
        Value::Object(object) => object.values().any(json_contains_applied_nominal_pattern),
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

fn has_builtin_call(calls: &[&Value], op: &str) -> bool {
    calls
        .iter()
        .any(|call| call["target"]["kind"] == "builtin" && call["target"]["op"] == op)
}

fn assert_native_call(
    executable: &Value,
    namespace: &str,
    symbol_name: &str,
    binding_key: &str,
) {
    let calls = call_exprs(executable);
    assert!(
        has_native_call(&calls, namespace, symbol_name, binding_key),
        "native call {namespace}.{symbol_name} ({binding_key}) should be present in {executable}"
    );
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

fn assert_record_fields(ty: &Value, expected: &[&str]) {
    assert_eq!(ty["kind"], "record", "expected record type: {ty}");
    let fields = ty["fields"]
        .as_object()
        .expect("record fields should be an object");
    let names = fields.keys().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(names, expected, "unexpected record fields: {ty}");
}

fn assert_nullable_profile_projection(ty: &Value) {
    assert_eq!(ty["kind"], "nullable");
    let profile = &ty["inner"];
    assert_record_fields(profile, &["displayName"]);
    assert_eq!(profile["fields"]["displayName"]["kind"], "builtin");
    assert_eq!(profile["fields"]["displayName"]["name"], "string");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_ir_emits_typed_slot_layout_and_refs() {
        let artifact = compile_package_file_ir(
            r#"
            function run(input: number) -> number {
                final total = input
                if true {
                    final total = 2
                    final copied = total
                }
                return total
            }
        "#,
            "internal/slots.skiff",
            "internal.slots",
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

        let outer_total = init_slot_stmt_by_slot(run, total_slots[0]);
        assert_eq!(
            load_slot(expr_for_ref(run, &outer_total["value"])),
            input_slot
        );

        let copied = init_slot_stmt_by_slot(run, copied_slot);
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
    fn string_cursor_comparison_lowers_to_db_predicate_descriptor() {
        let artifact = compile_package_file_ir(
            r#"
            type Credential { id: string, label: string }
            db object Credential { primary key(id) }

            function scan(lastId: string) -> Array<Credential> {
                return db find many Credential {
                  where id > lastId
                  order id asc
                  limit 100
                }
            }
        "#,
            "internal/string_cursor.skiff",
            "internal.string_cursor",
        )
        .expect("string cursor DB comparison should compile");
        let artifact_value = artifact.value();
        let scan = executable_entry(&artifact_value, "scan");
        let operations = db_operations(scan);
        let operation = db_operation(&operations, "find", true);
        let predicate = &operation["query"]["where"][0];
        let last_id_slot = slot_index(scan, "lastId", "param");

        assert_builtin_type(&scan["params"][0]["ty"], "string");
        assert_eq!(predicate["kind"], "compare");
        assert_eq!(predicate["field"]["segments"], serde_json::json!(["id"]));
        assert_eq!(predicate["op"], "gt");
        assert_eq!(
            load_slot(expr_for_ref(scan, &predicate["value"])),
            last_id_slot
        );
        assert!(
            scan["body"]["expressions"]
                .as_array()
                .expect("expressions should be an array")
                .iter()
                .all(|expression| expression["kind"] != "binary"),
            "DB string cursor must not lower to the runtime numeric binary path",
        );
    }

    #[test]
    fn same_scope_duplicate_final_is_rejected_before_ir_emission() {
        let error = compile_package_file_ir(
            r#"
            function run() -> number {
                final value = 1
                final value = 2
                return value
            }
        "#,
            "internal/duplicate.skiff",
            "internal.duplicate",
        )
        .expect_err("same-scope duplicate final should be a compiler error")
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
    fn package_file_ir_lowers_interface_box_with_expression_type_facts() {
        let artifact = compile_package_file_ir(
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
              final provider = Host{} as Provider
              return provider.name()
            }
        "#,
            "internal/interface_box_helper.skiff",
            "internal.interface_box_helper",
        )
        .expect("package File IR should lower interface boxing with expression facts");
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
            "package File IR should produce a local interface method table"
        );
    }

    #[test]
    fn static_callees_are_typed_call_targets_while_receiver_roots_are_slots() {
        let artifact = compile_package_file_ir(
            r#"
            function addOne(value: number) -> number {
                return value + 1
            }

            function run() -> number {
                final result = addOne(1)
                final second = internal.callees.addOne(result)
                var list: Array<number> = Array.empty<number>()
                list.push(second)
                return list.length()
            }
        "#,
            "internal/callees.skiff",
            "internal.callees",
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
            has_builtin_call(&calls, "Array.empty"),
            "Array.empty should lower to the compiler builtin op"
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
        let artifact = compile_package_file_ir(
            r#"
            function run() -> number {
                var items: Array<string> = Array.empty<string>()
                items.push("ok")
                final joined = string.join(items, ",")
                final parsed = number.parse("1")
                final body: bytes = bytes.fromUtf8(joined)
                return body.length()
            }
        "#,
            "internal/native_aliases.skiff",
            "internal.native_aliases",
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
    fn bytes_from_base64_lowers_to_exact_native_binding() {
        let artifact = compile_package_file_ir(
            r#"
            function jwtPayload(value: string) -> bytes {
                return bytes.fromBase64(value)
            }
        "#,
            "internal/base64.skiff",
            "internal.base64",
        )
        .expect("Base64 decoder fixture should compile");
        let artifact_value = artifact.value();
        let callable = executable_entry(&artifact_value, "jwtPayload");
        let calls = call_exprs(callable);

        assert!(
            has_native_call(&calls, "std.bytes", "fromBase64", "core.bytes.fromBase64"),
            "bytes.fromBase64 should lower through the exact canonical native binding"
        );

        for (name, source, expected) in [
            (
                "missing_argument",
                r#"
                function run() -> bytes {
                    return bytes.fromBase64()
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "extra_argument",
                r#"
                function run() -> bytes {
                    return bytes.fromBase64("YQ==", "Yg==")
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "wrong_argument",
                r#"
                function run() -> bytes {
                    return bytes.fromBase64(1)
                }
            "#,
                "call `bytes.fromBase64` argument 1",
            ),
            (
                "wrong_return",
                r#"
                function run() -> string {
                    return bytes.fromBase64("YQ==")
                }
            "#,
                "return type mismatch",
            ),
        ] {
            let error = compile_package_file_ir(
                source,
                format!("internal/base64_{name}.skiff"),
                format!("internal.base64_{name}"),
            )
            .expect_err("invalid bytes.fromBase64 call must fail closed")
            .to_string();
            assert!(error.contains(expected), "unexpected {name} error: {error}");
        }
    }

    #[test]
    fn bytes_from_hex_lowers_to_exact_native_binding() {
        let artifact = compile_package_file_ir(
            r#"
            function exactChunk(value: string) -> bytes {
                return bytes.fromHex(value)
            }
        "#,
            "internal/hex.skiff",
            "internal.hex",
        )
        .expect("hex decoder fixture should compile");
        let artifact_value = artifact.value();
        let callable = executable_entry(&artifact_value, "exactChunk");
        let calls = call_exprs(callable);

        assert!(
            has_native_call(&calls, "std.bytes", "fromHex", "core.bytes.fromHex"),
            "bytes.fromHex should lower through the exact canonical native binding"
        );

        for (name, source, expected) in [
            (
                "missing_argument",
                r#"
                function run() -> bytes {
                    return bytes.fromHex()
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "extra_argument",
                r#"
                function run() -> bytes {
                    return bytes.fromHex("61", "62")
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "wrong_argument",
                r#"
                function run() -> bytes {
                    return bytes.fromHex(1)
                }
            "#,
                "call `bytes.fromHex` argument 1",
            ),
            (
                "wrong_return",
                r#"
                function run() -> string {
                    return bytes.fromHex("61")
                }
            "#,
                "return type mismatch",
            ),
        ] {
            let error = compile_package_file_ir(
                source,
                format!("internal/hex_{name}.skiff"),
                format!("internal.hex_{name}"),
            )
            .expect_err("invalid bytes.fromHex call must fail closed")
            .to_string();
            assert!(error.contains(expected), "unexpected {name} error: {error}");
        }
    }

    #[test]
    fn bytes_concat_lowers_to_exact_native_binding_and_rejects_malformed_calls() {
        let artifact = compile_package_file_ir(
            r#"
            function multipart(chunks: Array<bytes>) -> bytes {
                return bytes.concat(chunks)
            }
        "#,
            "internal/bytes_concat.skiff",
            "internal.bytes_concat",
        )
        .expect("bytes concat fixture should compile");
        let artifact_value = artifact.value();
        let callable = executable_entry(&artifact_value, "multipart");
        let calls = call_exprs(callable);

        assert!(
            has_native_call(&calls, "std.bytes", "concat", "core.bytes.concat"),
            "bytes.concat should lower through the exact canonical native binding"
        );

        for (name, source, expected) in [
            (
                "missing_argument",
                r#"
                function run() -> bytes {
                    return bytes.concat()
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "extra_argument",
                r#"
                function run(chunks: Array<bytes>) -> bytes {
                    return bytes.concat(chunks, chunks)
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "wrong_argument",
                r#"
                function run() -> bytes {
                    return bytes.concat(Array.empty<string>())
                }
            "#,
                "call `bytes.concat` argument 1",
            ),
            (
                "wrong_return",
                r#"
                function run(chunks: Array<bytes>) -> string {
                    return bytes.concat(chunks)
                }
            "#,
                "return type mismatch",
            ),
        ] {
            let error = compile_package_file_ir(
                source,
                format!("internal/bytes_concat_{name}.skiff"),
                format!("internal.bytes_concat_{name}"),
            )
            .expect_err("invalid bytes.concat call must fail closed")
            .to_string();
            assert!(error.contains(expected), "unexpected {name} error: {error}");
        }
    }

    #[test]
    fn std_http_json_lowers_to_exact_std_native_targets() {
        let artifact = compile_package_file_ir(
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
                final headers = Array.empty<std.http.HttpHeader>()
                return std.http.jsonWithHeaders(200, JsonOutput {
                  marker: "ok",
                  count: 2
                }, headers)
            }
        "#,
            "internal/http_json_type_args.skiff",
            "internal.http_json_type_args",
        )
        .expect(
            "std.http JSON response helpers should infer native type args from record payloads",
        );
        let artifact_value = artifact.value();
        let plain = executable_entry(&artifact_value, "plain");
        let with_headers = executable_entry(&artifact_value, "withHeaders");

        assert_native_call(plain, "std.http", "json", "std.http.response.json");
        assert_native_call(
            with_headers,
            "std.http",
            "jsonWithHeaders",
            "std.http.response.jsonWithHeaders",
        );
    }

    #[test]
    fn receiver_mutation_and_assignment_lower_to_typed_targets() {
        let artifact = compile_package_file_ir(
            r#"
            type Session {
              players: Array<string>,
              title: string
            }

            function run(session: Session, memberId: string) -> number {
                var local = session
                local.players.push(memberId)
                local.title = "updated"
                return local.players.length()
            }
        "#,
            "internal/mutable_paths.skiff",
            "internal.mutable_paths",
        )
        .expect("mutable path fixture should compile");
        let artifact_value = artifact.value();
        let run = executable_entry(&artifact_value, "run");
        let local_slot = slot_index(run, "local", "local");

        let push = receiver_builtin_call(run, "Array", "push").expect("push call");
        let receiver = expr_for_ref(run, &push["args"][0]);
        assert_eq!(receiver["kind"], "field");
        assert_eq!(receiver["field"], "players");
        assert_eq!(
            load_slot(expr_for_ref(run, &receiver["object"])),
            local_slot
        );

        let assignment = find_stmt(run, |stmt| stmt["kind"] == "assign")
            .expect("session.title assignment should lower");
        assert_eq!(assignment["target"]["kind"], "field");
        assert_eq!(assignment["target"]["field"], "title");
        assert_eq!(
            load_slot(expr_for_ref(run, &assignment["target"]["object"])),
            local_slot
        );
    }

    #[test]
    fn user_impl_receiver_call_lowers_to_static_executable() {
        let artifact = compile_package_file_ir(
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
        let artifact = compile_package_file_ir(
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
        )
        .expect("generic impl receiver fixture should compile");
        let artifact_value = artifact.value();
        let method_index = executable_index(&artifact_value, "Box<T>.unwrap");
        let method = executable_entry(&artifact_value, "Box<T>.unwrap");
        assert_eq!(method["typeParams"], serde_json::json!(["T"]));
        let run = executable_entry(&artifact_value, "run");

        let call = call_exprs(run)
            .into_iter()
            .find(|call| {
                call["target"]["kind"] == "localExecutable"
                    && call["target"]["executableIndex"].as_u64() == Some(method_index)
            })
            .expect("generic impl receiver call should lower to the impl method executable");
        assert_eq!(
        call["typeArgs"]["T0"],
        serde_json::json!({
            "kind": "builtin",
            "name": "string"
        }),
        "Box<string>.unwrap must instantiate Box<T>.unwrap with the exact typed receiver argument"
    );
        assert!(
            dynamic_receiver_call(run, "unwrap").is_none(),
            "generic impl receiver call must not lower to DynamicReceiver"
        );
    }

    #[test]
    fn ordinary_erased_object_receiver_call_is_rejected_before_dynamic_receiver() {
        let error = compile_package_file_ir(
            r#"
            function run(item: {}) -> number {
                return item.length()
            }
        "#,
            "internal/erased_receiver.skiff",
            "internal.erased_receiver",
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
        let artifact = compile_package_file_ir(
            r#"
            function run(item: JsonObject) -> Json {
                return item.get("name")
            }
        "#,
            "internal/json_object_receiver.skiff",
            "internal.json_object_receiver",
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
    fn json_object_get_enforces_exact_receiver_arity_key_and_return_type() {
        for (name, source, expected) in [
            (
                "wrong_receiver",
                r#"
                function run(value: string) -> Json {
                    return value.get("field")
                }
            "#,
                "receiver method `get`",
            ),
            (
                "missing_argument",
                r#"
                function run(value: JsonObject) -> Json {
                    return value.get()
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "extra_argument",
                r#"
                function run(value: JsonObject) -> Json {
                    return value.get("field", "other")
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "wrong_key",
                r#"
                function run(value: JsonObject) -> Json {
                    return value.get(1)
                }
            "#,
                "call `JsonObject.get` argument 1",
            ),
            (
                "wrong_return",
                r#"
                function run(value: JsonObject) -> bool {
                    return value.get("field")
                }
            "#,
                "return type mismatch",
            ),
        ] {
            let error = compile_package_file_ir(
                source,
                format!("internal/json_object_get_{name}.skiff"),
                format!("internal.json_object_get_{name}"),
            )
            .expect_err("invalid JsonObject.get call must fail closed")
            .to_string();
            assert!(error.contains(expected), "unexpected {name} error: {error}");
        }
    }

    #[test]
    fn json_object_has_enforces_exact_receiver_and_arity() {
        let artifact = compile_package_file_ir(
            r#"
            function jsonObjectField(value: JsonObject, field: string) -> bool {
                return value.has(field)
            }
        "#,
            "internal/json_object_has.skiff",
            "internal.json_object_has",
        )
        .expect("JsonObject.has with one string argument should compile");
        let artifact_value = artifact.value();
        let function = executable_entry(&artifact_value, "jsonObjectField");
        assert!(
            receiver_builtin_call(function, "JsonObject", "has").is_some(),
            "JsonObject.has should lower to its exact receiver builtin target"
        );

        for (name, source, expected) in [
            (
                "wrong_receiver",
                r#"
                function run(value: string) -> bool {
                    return value.has("field")
                }
            "#,
                "receiver method `has`",
            ),
            (
                "missing_argument",
                r#"
                function run(value: JsonObject) -> bool {
                    return value.has()
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "extra_argument",
                r#"
                function run(value: JsonObject) -> bool {
                    return value.has("field", "other")
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "wrong_argument",
                r#"
                function run(value: JsonObject) -> bool {
                    return value.has(1)
                }
            "#,
                "call `JsonObject.has` argument 1",
            ),
        ] {
            let error = compile_package_file_ir(
                source,
                format!("internal/json_object_has_{name}.skiff"),
                format!("internal.json_object_has_{name}"),
            )
            .expect_err("invalid JsonObject.has call must fail closed")
            .to_string();
            assert!(error.contains(expected), "unexpected {name} error: {error}");
        }
    }

    #[test]
    fn map_has_and_set_enforce_generic_key_value_and_return_types() {
        let artifact = compile_package_file_ir(
            r#"
            type Item { value: string }

            function run(items: Map<string, Item>, key: string, value: Item) -> bool {
                var local = items
                local.set(key, value)
                return local.has(key)
            }
        "#,
            "internal/map_has_set.skiff",
            "internal.map_has_set",
        )
        .expect("well-typed Map.has/set calls should compile");
        let artifact_value = artifact.value();
        let function = executable_entry(&artifact_value, "run");
        assert!(receiver_builtin_call(function, "Map", "has").is_some());
        assert!(receiver_builtin_call(function, "Map", "set").is_some());

        for (name, source, expected) in [
            (
                "has_wrong_key",
                r#"
                function run(value: Map<string, number>) -> bool {
                    return value.has(1)
                }
            "#,
                "call `Map.has` argument 1",
            ),
            (
                "has_extra",
                r#"
                function run(value: Map<string, number>) -> bool {
                    return value.has("key", "extra")
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "set_wrong_key",
                r#"
                function run(value: Map<string, number>) -> void {
                    value.set(1, 2)
                }
            "#,
                "call `Map.set` argument 1",
            ),
            (
                "set_wrong_value",
                r#"
                function run(value: Map<string, number>) -> void {
                    value.set("key", "value")
                }
            "#,
                "call `Map.set` argument 2",
            ),
            (
                "set_missing",
                r#"
                function run(value: Map<string, number>) -> void {
                    value.set("key")
                }
            "#,
                "expected 2 arguments",
            ),
            (
                "has_wrong_return",
                r#"
                function run(value: Map<string, number>) -> string {
                    return value.has("key")
                }
            "#,
                "return type mismatch",
            ),
        ] {
            let error = compile_package_file_ir(
                source,
                format!("internal/map_has_set_{name}.skiff"),
                format!("internal.map_has_set_{name}"),
            )
            .expect_err("invalid Map.has/set call must fail closed")
            .to_string();
            assert!(error.contains(expected), "unexpected {name} error: {error}");
        }
    }

    #[test]
    fn json_object_delete_enforces_exact_receiver_key_and_return() {
        let artifact = compile_package_file_ir(
            r#"
            function removeField(value: JsonObject, field: string) -> bool {
                var local = value
                return local.delete(field)
            }
        "#,
            "internal/json_object_delete.skiff",
            "internal.json_object_delete",
        )
        .expect("JsonObject.delete with one string argument should compile");
        let artifact_value = artifact.value();
        let function = executable_entry(&artifact_value, "removeField");
        assert!(
            receiver_builtin_call(function, "JsonObject", "delete").is_some(),
            "JsonObject.delete should lower to its exact receiver builtin target"
        );

        for (name, source, expected) in [
            (
                "wrong_receiver",
                r#"
                function run(value: string) -> bool {
                    return value.delete("field")
                }
            "#,
                "receiver method `delete`",
            ),
            (
                "missing_argument",
                r#"
                function run(value: JsonObject) -> bool {
                    return value.delete()
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "extra_argument",
                r#"
                function run(value: JsonObject) -> bool {
                    return value.delete("field", "other")
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "wrong_key",
                r#"
                function run(value: JsonObject) -> bool {
                    return value.delete(1)
                }
            "#,
                "call `JsonObject.delete` argument 1",
            ),
            (
                "wrong_return",
                r#"
                function run(value: JsonObject) -> string {
                    return value.delete("field")
                }
            "#,
                "return type mismatch",
            ),
        ] {
            let error = compile_package_file_ir(
                source,
                format!("internal/json_object_delete_{name}.skiff"),
                format!("internal.json_object_delete_{name}"),
            )
            .expect_err("invalid JsonObject.delete call must fail closed")
            .to_string();
            assert!(error.contains(expected), "unexpected {name} error: {error}");
        }
    }

    #[test]
    fn explicitly_typed_json_object_local_in_transaction_lowers_set_to_receiver_builtin() {
        let artifact = compile_package_file_ir(
            r#"
            alias Metadata = JsonObject

            function run(runId: string, sourceRunId: string) -> void {
                db transaction {
                    var successorMetadata: Metadata = {
                      successorOf: runId,
                      reason: "failed-with-new-input",
                    }
                    successorMetadata.set("runtimeBindingsSourceRunId", sourceRunId)
                }
            }
        "#,
            "internal/drain_checkpoint_store.skiff",
            "internal.drain_checkpoint_store",
        )
        .expect("drain-checkpoint-shaped JsonObject.set fixture should compile");
        let artifact_value = artifact.value();
        let run = executable_entry(&artifact_value, "run");

        assert!(
            receiver_builtin_call(run, "JsonObject", "set").is_some(),
            "explicit JsonObject local must lower set to the exact JsonObject receiver builtin"
        );
        assert!(
            receiver_builtin_call(run, "Map", "set").is_none(),
            "JsonObject.set must not be selected through a method-name-only Map fallback"
        );
    }

    #[test]
    fn chained_string_receiver_call_lowers_to_receiver_builtin() {
        let artifact = compile_package_file_ir(
            r#"
            type SessionConfig {
              cookieName: string
            }

            function run(settings: SessionConfig, token: string) -> string {
                final value = settings.cookieName.concat("=")
                return value.concat(token)
            }
        "#,
            "internal/chained_string_receiver.skiff",
            "internal.chained_string_receiver",
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
    fn package_string_receiver_facts_flow_through_config_and_db_body() {
        let temp = TestDir::new("skiff-runtime-slots", "db-body-string-receiver");
        fs::create_dir_all(temp.path().join("internal")).unwrap();
        fs::write(
            temp.path().join("package.yml"),
            r#"
id: example.com/example
version: 1.0.0
"#,
        )
        .unwrap();
        fs::write(temp.path().join("api.yml"), "{}\n").unwrap();
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
                final marker = config.require<string>("runtimeLive.db")
                final prefix = "runtime-live-db-".concat(std.crypto.uuidSimple())
                final firstId = prefix.concat("-a")
                db insert RuntimeLiveDoc { id = firstId value = marker.concat("-first") visits = 1 rank = 10 }
                return firstId.contains(marker)
            }
        "#,
    )
    .unwrap();

        let project = compile_package_project(temp.path()).expect("package fixture should compile");
        let artifact = module_artifact(&project.package, "internal.db_receiver");
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
    fn string_contains_enforces_exact_receiver_and_arity() {
        let artifact = compile_package_file_ir(
            r#"
            function run(value: string) -> bool {
                return value.contains("@")
            }
        "#,
            "internal/string_contains.skiff",
            "internal.string_contains",
        )
        .expect("string.contains with one string argument should compile");
        let artifact_value = artifact.value();
        let run = executable_entry(&artifact_value, "run");
        assert!(
            receiver_builtin_call(run, "string", "contains").is_some(),
            "string.contains should lower to its exact receiver builtin target"
        );

        for (name, source, expected) in [
            (
                "wrong_receiver",
                r#"
                function run(value: number) -> bool {
                    return value.contains("@")
                }
            "#,
                "receiver method `contains`",
            ),
            (
                "missing_argument",
                r#"
                function run(value: string) -> bool {
                    return value.contains()
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "extra_argument",
                r#"
                function run(value: string) -> bool {
                    return value.contains("@", ".")
                }
            "#,
                "expected 1 arguments",
            ),
            (
                "wrong_argument",
                r#"
                function run(value: string) -> bool {
                    return value.contains(1)
                }
            "#,
                "call `string.contains` argument 1",
            ),
        ] {
            let error = compile_package_file_ir(
                source,
                format!("internal/string_contains_{name}.skiff"),
                format!("internal.string_contains_{name}"),
            )
            .expect_err("invalid string.contains call must fail closed")
            .to_string();
            assert!(error.contains(expected), "unexpected {name} error: {error}");
        }
    }

    #[test]
    fn array_empty_binding_receiver_call_lowers_to_receiver_builtin() {
        let artifact = compile_package_file_ir(
            r#"
            type Event {
              id: string
            }

            function run(event: Event) -> Array<Event> {
                var events = Array.empty<Event>()
                events.push(event)
                return events
            }
        "#,
            "internal/array_empty_receiver.skiff",
            "internal.array_empty_receiver",
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
    fn union_element_array_push_lowers_to_receiver_builtin() {
        let project = compile_root_alias_array_push(
            r#"
            function run() -> Array<root.types.Modality> {
                var items = Array.empty<root.types.Modality>()
                items.push("text")
                items.push("image")
                return items
            }
        "#,
        )
        .expect("Array<root alias literal union>.push should compile");
        let artifact = module_artifact(&project.package, "types_overlay");
        let artifact_value = artifact.value();
        let run = executable_entry(&artifact_value, "run");

        assert_eq!(
            call_exprs(run)
                .iter()
                .filter(|call| receiver_builtin_call_matches(call, "Array", "push"))
                .count(),
            2,
            "each union element Array.push should lower to receiverBuiltin",
        );
    }

    #[test]
    fn union_element_array_push_rejects_nonmembers_and_wrong_receivers() {
        let nonmember = compile_root_alias_array_push(
            r#"
            function run() -> Array<root.types.Modality> {
                var items = Array.empty<root.types.Modality>()
                items.push("document")
                return items
            }
        "#,
        )
        .expect_err("Array literal-union push must reject nonmembers")
        .to_string();
        assert!(
            nonmember.contains("Array.push")
                && nonmember.contains("argument 1")
                && nonmember.contains("document"),
            "nonmember error should identify the exact push argument, got:\n{nonmember}",
        );

        let wrong_receiver = compile_root_alias_array_push(
            r#"
            function run() -> number {
                final value: number = 1
                value.push("text")
                return value
            }
        "#,
        )
        .expect_err("non-array receivers must not acquire Array.push")
        .to_string();
        assert!(
            wrong_receiver.contains("receiver method `push`")
                && wrong_receiver.contains("number")
                && wrong_receiver.contains("must resolve"),
            "wrong receiver error should stay fail closed, got:\n{wrong_receiver}",
        );
    }

    #[test]
    fn literal_string_binding_receiver_call_lowers_to_receiver_builtin() {
        let artifact = compile_package_file_ir(
            r#"
            function run(delta: string) -> string {
                var activeText = ""
                activeText = activeText.concat(delta)
                return activeText
            }
        "#,
            "internal/literal_string_receiver.skiff",
            "internal.literal_string_receiver",
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
        let artifact = compile_package_file_ir(
            r#"
            function run(value: string) -> string {
                return value.replaceAll("-", "_")
            }
        "#,
            "internal/string_replace_all_receiver.skiff",
            "internal.string_replace_all_receiver",
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
        let artifact = compile_package_file_ir(
            r#"
            type Chunk {
              value: bytes
            }

            function run(chunks: Stream<Chunk>) -> bool {
                for chunk in chunks {
                    final text = chunk.value.toUtf8String()
                    if text.contains("data:") {
                        return true
                    }
                }
                return false
            }
        "#,
            "internal/stream_contains_receiver.skiff",
            "internal.stream_contains_receiver",
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
    fn bytes_to_hex_lowers_to_exact_receiver_builtin_and_rejects_near_misses() {
        let artifact = compile_package_file_ir(
            r#"
            function encode(value: bytes) -> string {
                return value.toHex()
            }
        "#,
            "internal/bytes_to_hex_receiver.skiff",
            "internal.bytes_to_hex_receiver",
        )
        .expect("bytes.toHex receiver fixture should compile");
        let artifact_value = artifact.value();
        let encode = executable_entry(&artifact_value, "encode");
        let call = receiver_builtin_call(encode, "bytes", "toHex")
            .expect("bytes.toHex should lower to receiverBuiltin");
        assert_eq!(
            call["target"]["op"]["canonicalKey"],
            "receiver:bytes.toHex@1"
        );
        assert_eq!(call["target"]["op"]["signatureVersion"], 1);

        for (name, source, expected) in [
            (
                "wrong_receiver",
                r#"
                function run(value: string) -> string {
                    return value.toHex()
                }
            "#,
                "toHex",
            ),
            (
                "extra_argument",
                r#"
                function run(value: bytes) -> string {
                    return value.toHex(1)
                }
            "#,
                "expected 0 arguments",
            ),
            (
                "wrong_return",
                r#"
                function run(value: bytes) -> bytes {
                    return value.toHex()
                }
            "#,
                "return type mismatch",
            ),
        ] {
            let error = compile_package_file_ir(
                source,
                format!("internal/bytes_to_hex_{name}.skiff"),
                format!("internal.bytes_to_hex_{name}"),
            )
            .expect_err("invalid bytes.toHex call must fail closed")
            .to_string();
            assert!(error.contains(expected), "unexpected {name} error: {error}");
        }
    }

    #[test]
    fn std_http_body_bytes_receiver_chain_lowers_to_receiver_builtin() {
        let artifact = compile_package_file_ir(
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
    fn generic_nominal_match_pattern_emits_applied_nominal_type_pattern() {
        let artifact = compile_package_file_ir(
            r#"
            type Box<T> {
              value: T
            }

            function run(boxed: Box<string>) -> string {
                match boxed {
                  Box<string> { value } => {
                    return "matched"
                  }
                  _ => {
                    return "missing"
                  }
                }
            }
        "#,
            "internal/generic_nominal_pattern.skiff",
            "internal.generic_nominal_pattern",
        )
        .expect("generic nominal pattern should lower to PatternIr::Type");

        assert!(
            json_contains_applied_nominal_pattern(&artifact.value()),
            "generic nominal pattern must preserve ordered arguments in PatternIr::Type: {}",
            artifact.value()
        );
    }

    #[test]
    fn record_literal_and_binding_patterns_do_not_emit_type_pattern_ir() {
        let artifact = compile_package_file_ir(
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
        let artifact = compile_package_file_ir(
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
                final inserted = db insert User { id = "u1" name = "Ada" visits = 0 }
                final updated = db update User("u1") { visits += 1 }
                final replaced = db replace User("u1") { name = "Grace" visits = 2 }
                final upserted = db upsert User("u1") { name = "Ada" visits = 0 } { visits += 1 }
                final insertedMany = db insert many User values rows
                final updatedMany = db update many User { where name != null } { visits += 1 }
                final deletedMany = db delete many User { where name == "Ada" }
                return true
            }
        "#,
            "internal/db_write_results.skiff",
            "internal.db_write_results",
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
    fn object_db_projection_type_matches_source_access_and_runtime_descriptor() {
        let artifact = compile_package_file_ir(
        r#"
            import std

            type Profile {
              displayName: string,
              ignored: number
            }

            type Credential {
              id: string,
              profile: Profile?,
              apiKey: string
            }

            db object Credential {
              primary key(id)
              storage apiKey using encrypted
            }

            function projected(id: string) -> { id: string, apiKey: string } {
              final row = db require Credential(id) { fields { apiKey } }
              return { id: row.id, apiKey: row.apiKey }
            }

            function encoded(id: string) -> string {
              final row = db require Credential(id) { fields { apiKey } }
              return std.json.encode(row)
            }

            function projectedMany() -> Array<{ id: string, profile: { displayName: string }? }> {
              return db find many Credential { fields { profile.displayName } }
            }

            function projectedOptional(id: string) -> { id: string, profile: { displayName: string }? }? {
              return db optional Credential(id) { fields { profile.displayName } }
            }
        "#,
        "internal/db_projection_result.skiff",
        "internal.db_projection_result",
    )
    .expect("projected fields should be accessible and JSON encodable");
        let value = artifact.value();

        for name in ["projected", "encoded"] {
            let operations = db_operations(executable_entry(&value, name));
            let operation = db_operation(&operations, "require", false);
            assert_record_fields(&operation["resultType"], &["apiKey", "id"]);
        }

        let many_operations = db_operations(executable_entry(&value, "projectedMany"));
        let many = db_operation(&many_operations, "find", true);
        assert_eq!(many["resultType"]["kind"], "builtin");
        assert_eq!(many["resultType"]["name"], "Array");
        let many_item = &many["resultType"]["args"][0];
        assert_record_fields(many_item, &["id", "profile"]);
        assert_nullable_profile_projection(&many_item["fields"]["profile"]);

        let optional_operations = db_operations(executable_entry(&value, "projectedOptional"));
        let optional = db_operation(&optional_operations, "optional", false);
        assert_eq!(optional["resultType"]["kind"], "nullable");
        let optional_inner = &optional["resultType"]["inner"];
        assert_record_fields(optional_inner, &["id", "profile"]);
        assert_nullable_profile_projection(&optional_inner["fields"]["profile"]);
    }

    #[test]
    fn object_db_upsert_result_fields_lower_to_static_field_access() {
        let artifact = compile_package_file_ir(
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
                final result = db upsert User("u1") { name = "Ada" visits = 0 } { visits += 1 }
                final inserted = result.inserted
                final name = result.value.name
                final visits = result.value.visits
                if inserted {
                    return name == "Ada"
                }
                return visits == 1
            }
        "#,
            "internal/db_upsert_result_fields.skiff",
            "internal.db_upsert_result_fields",
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
                    final user = db insert User { id = "u1" name = "Ada" visits = 0 }
                    user.name = "Grace"
                    return true
                }
            "#,
                "assignment target derives from immutable binding `user`",
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
                    final user = db update User("u1") { visits += 1 }
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
                    final user = db replace User("u1") { name = "Grace" visits = 2 }
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
                    final result = db upsert User("u1") { name = "Ada" visits = 0 } { visits += 1 }
                    result.value.name = "Grace"
                    return true
                }
            "#,
                "assignment target derives from immutable binding `result`",
            ),
        ] {
            let error = compile_package_file_ir(
                source,
                format!("internal/db_write_readonly_{name}.skiff"),
                format!("internal.db_write_readonly_{name}"),
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
        let artifact = compile_package_file_ir(
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
                final ids: Array<UserId> = users.keys()
                for id in users {
                    final copy: UserId = keepUserId(id)
                }
                for id, user in users {
                    final copy: UserId = keepUserId(id)
                    final name: string = keepUser(user)
                }
                return ids
            }
        "#,
            "internal/map_for.skiff",
            "internal.map_for",
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
            let error = compile_package_file_ir(
                source,
                format!("internal/entry_for_{name}.skiff"),
                format!("internal.entry_for_{name}"),
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
                "assignment target derives from immutable binding `key`",
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
                "assignment target derives from immutable binding `value`",
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
            let error = compile_package_file_ir(
                source,
                format!("internal/map_for_{name}.skiff"),
                format!("internal.map_for_{name}"),
            )
            .expect_err("invalid map for binding should fail")
            .to_string();
            assert!(
                error.contains(expected),
                "unexpected map for binding error for {name}: {error}"
            );
        }
    }
}
