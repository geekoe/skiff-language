use crate::ast::{DbIndexDirection, DbRetentionUnit, ForBinding, Stmt};

use super::{
    parse_source, parse_source_metadata, parse_source_with_bodies_tolerant, BuiltinPackage,
    PackageId,
};

mod spawn;

#[test]
fn parses_single_and_entry_for_bindings() {
    let ast = parse_source(
        r#"
        function run(users: Map<string, string>) -> void {
          for key in users {
            return
          }
          for key, value in users {
            return
          }
        }
        "#,
    )
    .unwrap();
    let statements = &ast.functions[0].body.statements;

    match &statements[0] {
        Stmt::For {
            binding: ForBinding::Item { item },
            ..
        } => assert_eq!(item, "key"),
        other => panic!("expected single-binding for, got {other:?}"),
    }
    match &statements[1] {
        Stmt::For {
            binding: ForBinding::Entry { key, value },
            ..
        } => {
            assert_eq!(key, "key");
            assert_eq!(value, "value");
        }
        other => panic!("expected entry-binding for, got {other:?}"),
    }
}

#[test]
fn rejects_tuple_like_for_binding_syntax() {
    let error = parse_source(
        r#"
        function run(users: Map<string, string>) -> void {
          for (key, value) in users {
            return
          }
        }
        "#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("expected loop item name"),
        "unexpected parse error: {error}"
    );
}

#[test]
fn parses_service_interface_metadata_without_fixture() {
    let source = r#"
        type Club {
          id: ClubId,
          name: string?
        }

        type ClubId = string

        interface ClubSocket {
          function clubCreate(name: string) -> Club
          function userSetNickname(userId: UserId, nickname: string) -> Club
        }
    "#;
    let ast = parse_source(source).unwrap();

    assert!(!ast
        .interfaces
        .iter()
        .any(|interface| interface.name == "ClubConnection"));

    let socket = ast
        .interfaces
        .iter()
        .find(|interface| interface.name == "ClubSocket")
        .expect("ClubSocket interface");
    assert!(!socket.exported);
    assert!(socket
        .operations
        .iter()
        .any(|operation| operation.name == "clubCreate"));
    assert!(socket
        .operations
        .iter()
        .any(|operation| operation.name == "userSetNickname"));
    assert_eq!(socket.operations.len(), 2);

    let club = ast
        .types
        .iter()
        .find(|ty| ty.name == "Club")
        .expect("Club type");
    let club_name = club
        .fields
        .iter()
        .find(|field| field.name == "name")
        .expect("Club.name field");
    assert!(club_name.ty.name.contains("string?"));
}

#[test]
fn parses_service_implementation_metadata_without_fixture() {
    let source = r#"
        import mongo
        import std

        type ClubService implements root.api.club.ClubSocket {}

        function apiError(code: ApiErrorCode, message: string) -> ApiError {
          return ApiError { code: code, message: message }
        }

        function getUser(userId: UserId) -> User? {
          return null
        }

        impl ClubService {
          function connect(self: ClubService, connectionId: string) -> void {
            return
          }

          function clubCreate(self: ClubService, name: string) -> root.api.club.Club {
            return {}
          }

          function userSetNickname(self: ClubService, userId: UserId, nickname: string) -> root.api.club.Club {
            return {}
          }
        }
    "#;
    let ast = parse_source_metadata(source).unwrap();

    assert_eq!(ast.imports.len(), 2);
    let mongo_import = &ast.imports[0];
    assert_eq!(mongo_import.path, vec!["mongo"]);
    assert_eq!(mongo_import.local_binding.as_deref(), Some("mongo"));
    assert_eq!(
        mongo_import.package,
        Some(PackageId::Simple {
            name: "mongo".to_string()
        })
    );
    let std_import = &ast.imports[1];
    assert_eq!(std_import.path, vec!["std"]);
    assert_eq!(std_import.local_binding.as_deref(), Some("std"));
    assert_eq!(
        std_import.package,
        Some(PackageId::Builtin {
            name: BuiltinPackage::Std
        })
    );
    let service = ast
        .types
        .iter()
        .find(|ty| ty.name == "ClubService")
        .expect("ClubService type");
    assert!(!service
        .implements
        .iter()
        .any(|ty| ty.name == "root.api.club.ClubConnection"));
    assert!(service
        .implements
        .iter()
        .any(|ty| ty.name == "root.api.club.ClubSocket"));

    let impl_decl = ast
        .impls
        .iter()
        .find(|impl_decl| impl_decl.target == "ClubService")
        .expect("ClubService impl");
    assert_eq!(impl_decl.methods.len(), 3);
    for name in ["connect", "clubCreate", "userSetNickname"] {
        assert!(
            impl_decl.methods.iter().any(|method| method.name == name),
            "expected impl method {name}"
        );
    }

    for name in ["apiError", "getUser"] {
        assert!(
            ast.function_signatures
                .iter()
                .any(|function| function.name == name),
            "expected top-level function signature {name}"
        );
    }
}

#[test]
fn parses_transparent_alias_declaration() {
    let source = r#"
            alias UserIds = Array<UserId>
        "#;

    let ast = parse_source(source).unwrap();

    assert!(ast.types.is_empty());
    assert_eq!(ast.aliases.len(), 1);
    let alias = &ast.aliases[0];
    assert!(!alias.exported);
    assert_eq!(alias.name, "UserIds");
    assert_eq!(alias.target_type.name, "Array<UserId>");
}

#[test]
fn parses_db_declaration_with_key_indexes_and_contextual_keywords() {
    let source = r#"
            type Thread {
              id: ThreadId,
              ownerUserId: UserId,
              createdAt: Instant,
              externalId: string?
            }

            db object Thread {
              name "thread"
              primary key(id);
              index byOwner(ownerUserId, createdAt desc);
              unique index byExternalId(externalId) where externalId != null
            }

            function useContextualNames() -> number {
              const key = 1
              const index = 2
              const activate = 3
              return key + index + activate
            }
        "#;

    let ast = parse_source(source).unwrap();

    assert_eq!(ast.dbs.len(), 1);
    let db = &ast.dbs[0];
    assert_eq!(db.name, "Thread");
    assert_eq!(db.collection_name.as_deref(), Some("thread"));
    assert_eq!(db.retention, None);
    let key = db.key.as_ref().expect("db object key should parse");
    assert_eq!(key.name, "id");
    assert_eq!(db.indexes.len(), 2);
    assert_eq!(db.indexes[0].name, "byOwner");
    assert!(!db.indexes[0].unique);
    assert_eq!(db.indexes[0].fields[1].field_path, vec!["createdAt"]);
    assert_eq!(db.indexes[0].fields[1].direction, DbIndexDirection::Desc);
    assert_eq!(db.indexes[1].name, "byExternalId");
    assert!(db.indexes[1].unique);
    assert!(db.indexes[1].where_expr.is_some());
}

#[test]
fn rejects_db_object_stored_field_entry() {
    let error = parse_source(
        r#"
            db object Thread {
              name "thread"
              ownerUserId: UserId
              primary key(id)
            }
        "#,
    )
    .expect_err("db object fields should fail during parsing");

    assert!(
        error
            .to_string()
            .contains("db object stored fields must be declared on the attached type"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_db_object_duplicate_key_declaration() {
    let error = parse_source(
        r#"
            db object Thread {
              name "thread"
              primary key(id)
              primary key(otherId)
            }
        "#,
    )
    .expect_err("duplicate db object keys should fail during parsing");

    assert!(
        error
            .to_string()
            .contains("db object key is declared more than once"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_db_object_typed_key_entry() {
    let error = parse_source(
        r#"
            db object Thread {
              name "thread"
              key id: string
            }
        "#,
    )
    .expect_err("typed db object key should fail during parsing");

    assert!(
        error
            .to_string()
            .contains("db object key type belongs on the attached type"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_db_object_declaration_metadata_retention_and_indexes() {
    let source = r#"
            type Message {
              id: MessageId,
              threadId: ThreadId,
              timestamp: Instant
            }

            db object Message {
              name "message"
              primary key(id)
            }

            type Thread {
              id: ThreadId,
              ownerUserId: UserId
            }

            db object Thread {
              name "thread"
              primary key(id)
              retention 180 days
              index byOwner(ownerUserId desc, id desc)
            }
        "#;

    let ast = parse_source(source).unwrap();

    assert_eq!(ast.dbs.len(), 2);
    let db = ast
        .dbs
        .iter()
        .find(|db| db.name == "Thread")
        .expect("Thread db object");
    assert_eq!(db.collection_name.as_deref(), Some("thread"));
    let retention = db.retention.as_ref().expect("retention should parse");
    assert_eq!(retention.amount, 180);
    assert_eq!(retention.unit, DbRetentionUnit::Days);
    assert_eq!(db.indexes.len(), 1);
    assert_eq!(db.indexes[0].name, "byOwner");
    assert_eq!(db.indexes[0].fields[0].field_path, vec!["ownerUserId"]);
    assert_eq!(db.indexes[0].fields[0].direction, DbIndexDirection::Desc);
}

#[test]
fn rejects_db_object_relation_declaration_in_object_db_v1() {
    let error = parse_source(
        r#"
            db object Thread {
              name "thread"
              primary key(id)
              relation messages: Message many { match threadId }
            }
        "#,
    )
    .expect_err("object DB v1 should reject relation declarations");

    assert!(
        error
            .to_string()
            .contains("db object relation declarations are not supported in object DB v1"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_top_level_process_declaration() {
    let source = r#"
            type ThreadCoordinator {
              threadId: ThreadId,
            }

            process ThreadCoordinator(threadId: ThreadId) {
              activate -> ThreadCoordinator {
                return ThreadCoordinator { threadId: threadId }
              }

              consumer(self: ThreadCoordinator) {
                return
              }
            }
        "#;

    let error = parse_source(source).expect_err("top-level process declarations are removed");

    assert!(
        error.to_string().contains("process has been removed"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_process_expression() {
    let source = r#"
            function build(threadId: ThreadId) -> WorkerHandle {
              return process ThreadCoordinator(threadId, "first")
            }
        "#;

    let error = parse_source(source).expect_err("process expressions are removed");

    assert!(
        error.to_string().contains("process has been removed"),
        "unexpected error: {error}"
    );
}

// NOTE: the removed `wait until`, `notify`/`notify all`, and `continue consumer`
// statements are no longer parser keywords, so they parse as plain identifiers /
// expression statements and a bare `continue`. They are only rejected by later
// compilation (unresolved identifiers / `continue` outside a loop). Those cases are
// covered in projection::typed_artifacts::tests via compile_source_file_ir_unit.

#[test]
fn type_equals_stays_nominal_representation_declaration() {
    let source = r#"
            type UserIds = Array<UserId>
        "#;

    let ast = parse_source(source).unwrap();

    assert!(ast.aliases.is_empty());
    assert_eq!(ast.types.len(), 1);
    let ty = &ast.types[0];
    assert_eq!(ty.name, "UserIds");
    assert!(ty.type_params.is_empty());
    assert_eq!(
        ty.alias.as_ref().map(|target| target.name.as_str()),
        Some("Array<UserId>")
    );
}

#[test]
fn parses_type_discriminator_declaration() {
    let source = r#"
            type Result discriminator "kind" =
              { kind: "ok", value: string }
              | { kind: "err", message: string }
        "#;

    let ast = parse_source(source).unwrap();

    assert_eq!(ast.types.len(), 1);
    let ty = &ast.types[0];
    assert!(!ty.exported);
    assert_eq!(ty.name, "Result");
    assert!(ty.type_params.is_empty());
    assert_eq!(ty.discriminator.as_deref(), Some("kind"));
    assert_eq!(
        ty.alias.as_ref().map(|target| target.name.as_str()),
        Some(r#"{ kind: "ok", value: string } | { kind: "err", message: string }"#)
    );
}

#[test]
fn parses_generic_type_declaration_params() {
    let source = r#"
            type Result<T> discriminator "kind" =
              { kind: "ok", value: T }
              | { kind: "err", message: string }
        "#;

    let ast = parse_source(source).unwrap();

    assert_eq!(ast.types.len(), 1);
    let ty = &ast.types[0];
    assert_eq!(ty.name, "Result");
    assert_eq!(ty.type_params, vec!["T"]);
    assert_eq!(ty.discriminator.as_deref(), Some("kind"));
}

#[test]
fn rejects_anonymous_record_union_without_discriminator() {
    let source = r#"
            type Result =
              { tag: "ok", value: string }
              | { tag: "err", message: string }
        "#;

    let error = parse_source(source).unwrap_err();

    assert!(error.to_string().contains("named union type Result"));
    assert!(error.to_string().contains("discriminator \"tag\""));
}

#[test]
fn rejects_discriminator_without_anonymous_record_union_branches() {
    for source in [
        r#"
            type Id discriminator "kind" = string
        "#,
        r#"
            type Event discriminator "kind" = TextEvent | CountEvent
        "#,
    ] {
        let error = parse_source(source).unwrap_err();

        assert!(error
            .to_string()
            .contains("discriminator can only be used with anonymous record union branches"));
    }
}

#[test]
fn rejects_malformed_alias_declarations() {
    for (source, expected) in [
        ("alias UserIds Array<UserId>", "expected symbol ="),
        ("alias = Array<UserId>", "expected alias name"),
    ] {
        let error = parse_source(source).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?}, got: {error}"
        );
    }
}

#[test]
fn parses_impl_method_bodies_tolerantly_without_fixture() {
    let source = r#"
        type ClubService implements root.api.club.ClubSocket {}

        impl ClubService {
          function connect(self: ClubService, connectionId: string) -> void {
            return
          }

          function receive(self: ClubService, connectionId: string, frame: string) -> void {
            const accepted = true
            if accepted {
              return
            }
            return
          }

          function clubCreate(self: ClubService, name: string) -> root.api.club.Club {
            return {}
          }
        }

        function getMyMember(clubId: ClubId, userId: UserId) -> Member? {
          return null
        }

        function requireClubAdmin(member: Member) -> void {
          return
        }
    "#;
    let ast = parse_source_with_bodies_tolerant(source).unwrap();

    let impl_decl = ast
        .impls
        .iter()
        .find(|impl_decl| impl_decl.target == "ClubService")
        .expect("ClubService impl");

    assert_eq!(impl_decl.methods.len(), 3);
    for name in ["connect", "receive", "clubCreate"] {
        assert!(
            impl_decl.methods.iter().any(|method| method.name == name),
            "expected impl method signature {name}"
        );
    }

    for name in ["connect", "receive", "clubCreate"] {
        let method = impl_decl
            .method_bodies
            .iter()
            .find(|method| method.name == name)
            .unwrap_or_else(|| panic!("expected impl method body {name}"));
        assert!(
            !method.body.statements.is_empty(),
            "expected non-empty body for {name}"
        );
    }

    for name in ["getMyMember", "requireClubAdmin"] {
        let function = ast
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap_or_else(|| panic!("expected helper function body {name}"));
        assert!(
            !function.body.statements.is_empty(),
            "expected non-empty body for {name}"
        );
    }
}

#[test]
fn parses_else_if_as_nested_if_in_tolerant_impl_body() {
    let source = r#"
            impl ExampleImpl {
              function choose(self: ExampleImpl, flag: boolean, fallback: boolean) -> Output {
                if flag {
                  return {}
                } else if fallback {
                  return {}
                } else {
                  return {}
                }
              }
            }
        "#;
    let ast = parse_source_with_bodies_tolerant(source).unwrap();
    let method = ast.impls[0]
        .method_bodies
        .iter()
        .find(|method| method.name == "choose")
        .unwrap();
    let crate::ast::Stmt::If {
        else_block: Some(else_block),
        ..
    } = &method.body.statements[0]
    else {
        panic!("expected outer if with else block");
    };
    assert!(matches!(
        else_block.statements.first(),
        Some(crate::ast::Stmt::If { .. })
    ));
}

#[test]
fn full_parse_populates_impl_method_bodies() {
    let source = r#"
            impl Example {
              function touch(self: Example, value: string) -> string {
                return value
              }
            }
        "#;

    let ast = parse_source(source).unwrap();
    let implementation = &ast.impls[0];

    assert_eq!(implementation.methods.len(), 1);
    assert_eq!(implementation.method_bodies.len(), 1);
    assert_eq!(implementation.methods[0].name, "touch");
    assert_eq!(implementation.method_bodies[0].name, "touch");
    assert!(matches!(
        implementation.method_bodies[0].body.statements.first(),
        Some(crate::ast::Stmt::Return(Some(_)))
    ));
}

#[test]
fn parses_bare_return_statement() {
    let source = r#"
            function done() -> void {
              return
            }
        "#;

    let ast = parse_source(source).unwrap();
    let statements = &ast.functions[0].body.statements;

    assert!(matches!(
        statements.first(),
        Some(crate::ast::Stmt::Return(None))
    ));
}

#[test]
fn bodies_tolerant_keeps_top_level_signature_when_body_fails() {
    let source = r#"
            function broken() -> number {
                assert true
            }

            function ok() -> number {
                return 2
            }
        "#;

    let ast = parse_source_with_bodies_tolerant(source).unwrap();

    assert!(ast.functions.iter().any(|function| function.name == "ok"));
    assert!(!ast
        .functions
        .iter()
        .any(|function| function.name == "broken"));
    assert!(ast
        .function_signatures
        .iter()
        .any(|function| function.name == "broken"));
}

#[test]
fn bodies_tolerant_keeps_impl_signature_when_body_fails() {
    let source = r#"
            impl Example {
                function broken() -> number {
                    assert true
                }
            }
        "#;

    let ast = parse_source_with_bodies_tolerant(source).unwrap();
    let implementation = &ast.impls[0];
    let broken = implementation
        .methods
        .iter()
        .find(|method| method.name == "broken")
        .unwrap();

    assert!(implementation.method_bodies.is_empty());
    assert_eq!(
        broken.implicit_self.as_ref().map(|ty| ty.name.as_str()),
        Some("Example")
    );
}

#[test]
fn parses_native_function_without_body() {
    let source = r#"
            native function hostParse<T>(value: string) -> number

            function main() -> number {
                return 1
            }
        "#;

    let ast = parse_source(source).unwrap();

    assert_eq!(ast.functions.len(), 2);
    let native = &ast.functions[0];
    assert_eq!(native.name, "hostParse");
    assert!(native.is_native);
    assert_eq!(native.type_params, vec!["T"]);
    assert!(native.body.statements.is_empty());

    let main = &ast.functions[1];
    assert_eq!(main.name, "main");
    assert!(!main.is_native);
}

#[test]
fn parses_static_native_impl_methods_and_implicit_self() {
    let source = r#"
            impl Array<T> {
                native static function empty<T>() -> Array<T>
                native function length() -> number
                function explicit(self: Array<T>) -> number {
                    return self.length()
                }
                function implicit() -> number {
                    return self.length()
                }
            }
        "#;

    let ast = parse_source_with_bodies_tolerant(source).unwrap();
    let implementation = &ast.impls[0];

    assert_eq!(implementation.target, "Array<T>");

    let empty = implementation
        .methods
        .iter()
        .find(|method| method.name == "empty")
        .unwrap();
    assert!(empty.is_native);
    assert!(empty.is_static);
    assert!(empty.implicit_self.is_none());
    assert_eq!(empty.type_params, vec!["T"]);

    let length = implementation
        .methods
        .iter()
        .find(|method| method.name == "length")
        .unwrap();
    assert!(length.is_native);
    assert!(!length.is_static);
    assert_eq!(
        length.implicit_self.as_ref().map(|ty| ty.name.as_str()),
        Some("Array<T>")
    );

    let explicit = implementation
        .method_bodies
        .iter()
        .find(|method| method.name == "explicit")
        .unwrap();
    assert!(explicit.implicit_self.is_none());

    let implicit = implementation
        .method_bodies
        .iter()
        .find(|method| method.name == "implicit")
        .unwrap();
    assert_eq!(
        implicit.implicit_self.as_ref().map(|ty| ty.name.as_str()),
        Some("Array<T>")
    );
}

#[test]
fn rejects_provider_body_in_tolerant_mode() {
    let source = r#"
            provider app.live

            provider function hostValue() -> string {
                return "x"
            }
        "#;

    let error = parse_source_with_bodies_tolerant(source).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("legacy provider syntax has been removed"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_function_type_in_native_method_signature() {
    let source = r#"
            impl Array<T> {
                native function map<R>(f: fn(item: T) -> R) -> Array<R>
            }
        "#;

    let ast = parse_source_metadata(source).unwrap();
    let method = &ast.impls[0].methods[0];

    assert_eq!(method.name, "map");
    assert_eq!(method.type_params, vec!["R"]);
    assert_eq!(method.params[0].ty.name, "fn(item: T) -> R");
    assert_eq!(method.return_type.name, "Array<R>");
}

#[test]
fn parses_throw_rethrow_and_catch_expressions() {
    let source = r#"
            function run(error: LoginError, exception: Exception<LoginError>) -> CatchResult<number, LoginError> {
                const result = catch<LoginError>(compute(throw error))
                rethrow exception
            }
        "#;

    let ast = parse_source(source).unwrap();
    let statements = &ast.functions[0].body.statements;

    let crate::ast::Stmt::Let { value, .. } = &statements[0] else {
        panic!("expected catch binding");
    };
    let crate::ast::Expr::Catch {
        catch_type,
        try_expr,
    } = value
    else {
        panic!("expected catch expression");
    };
    assert_eq!(catch_type.name, "LoginError");

    let crate::ast::Expr::Call { args, .. } = try_expr.as_ref() else {
        panic!("expected catch try call expression");
    };
    assert!(matches!(args.first(), Some(crate::ast::Expr::Throw { .. })));

    assert!(matches!(statements[1], crate::ast::Stmt::Rethrow { .. }));
}

#[test]
fn parses_throw_statement() {
    let source = r#"
            function fail(error: LoginError) -> never {
                throw error
            }
        "#;

    let ast = parse_source(source).unwrap();

    assert!(matches!(
        ast.functions[0].body.statements[0],
        crate::ast::Stmt::Throw { .. }
    ));
}

#[test]
fn parses_object_literal_bare_key() {
    let source = r#"
            function build(value: string) -> Value {
                return { name: value }
            }
        "#;

    let ast = parse_source(source).unwrap();
    let crate::ast::Stmt::Return(Some(crate::ast::Expr::ObjectLiteral { entries })) =
        &ast.functions[0].body.statements[0]
    else {
        panic!("expected object literal return");
    };

    assert_eq!(entries.len(), 1);
    assert!(matches!(
        &entries[0].key,
        crate::ast::ObjectLiteralKey::Name(name) if name == "name"
    ));
    assert_eq!(
        entries[0].key_span.map(|span| span.start.offset),
        source.find("name")
    );
}

#[test]
fn parses_typed_patch_expression() {
    let source = r#"
            type User { id: string, name: string, visits: number }

            function build(now: number) -> Value {
                return patch<User> {
                    set name = "Ada";
                    inc stats.visits by 1
                    set lastSeenAt = now
                }
            }
        "#;

    let ast = parse_source(source).unwrap();
    let crate::ast::Stmt::Return(Some(crate::ast::Expr::Patch { target, operations })) =
        &ast.functions[0].body.statements[0]
    else {
        panic!("expected typed patch return");
    };

    assert_eq!(target.name, "User");
    assert_eq!(operations.len(), 3);
    assert!(matches!(
        &operations[0],
        crate::ast::PatchOperation::Set { path, value: crate::ast::Expr::Literal(crate::ast::Literal::String(value)) }
            if path == &vec!["name".to_string()] && value == "Ada"
    ));
    assert!(matches!(
        &operations[1],
        crate::ast::PatchOperation::Inc { path, value: crate::ast::Expr::Literal(crate::ast::Literal::Number(value)) }
            if path == &vec!["stats".to_string(), "visits".to_string()] && (*value - 1.0).abs() < f64::EPSILON
    ));
    assert!(matches!(
        &operations[2],
        crate::ast::PatchOperation::Set { path, value: crate::ast::Expr::Identifier(name) }
            if path == &vec!["lastSeenAt".to_string()] && name == "now"
    ));
}

#[test]
fn rejects_object_literal_computed_key() {
    let source = r#"
            function build(key: string, value: string) -> Value {
                return { [key]: value }
            }
        "#;

    let error = parse_source(source).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("computed object literal keys are not supported"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_object_literal_string_key() {
    let source = r#"
            function build(items: Items) -> Value {
                return { "$or": items }
            }
        "#;

    let error = parse_source(source).unwrap_err();

    assert!(
        error.to_string().contains("expected object literal key"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_record_construct_string_or_computed_keys() {
    let string_key = r#"
            function build() -> User {
                return User { "name": "Ada" }
            }
        "#;
    let computed_key = r#"
            function build(key: string) -> User {
                return User { [key]: "Ada" }
            }
        "#;

    for source in [string_key, computed_key] {
        let error = parse_source(source).unwrap_err();
        assert!(
            error.to_string().contains("expected record field name"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn parses_source_file_tests() {
    let source = r#"
            function helper() -> number { return 1 }

            test defaultRun false

            test "build body" {
              assert helper() == 1, "helper should return 1"
            }

            test "more" {
              assert true
            }
        "#;

    let ast = parse_source_with_bodies_tolerant(source).unwrap();
    assert_eq!(ast.tests.len(), 2);
    assert_eq!(ast.tests[0].name, "build body");
    assert_eq!(ast.tests[1].name, "more");
    assert_eq!(ast.test_default_run, Some(false));
    assert!(matches!(
        ast.tests[0].body.statements[0],
        crate::ast::Stmt::Assert {
            condition: _,
            message: _
        }
    ));
}

#[test]
fn rejects_assert_outside_test_blocks() {
    let source = r#"assert true"#;

    let error = parse_source(source).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("assert can only be used in test blocks"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_db_transaction_expressions() {
    let source = r#"
            function run(id: string) -> string {
              db transaction {
                const user = db require User(id)
                db insert User { id = id name = "Ada" }
              }
              const committed = db transaction value { id }
              return id
            }
        "#;

    let ast = parse_source(source).unwrap();
    let statements = &ast.functions[0].body.statements;
    let crate::ast::Stmt::Expr(crate::ast::Expr::DbTransaction(transaction)) = &statements[0]
    else {
        panic!(
            "expected db transaction expression statement, got {:?}",
            statements[0]
        );
    };
    assert_eq!(transaction.mode, crate::ast::DbBlockMode::Effect);
    assert_eq!(transaction.body.statements.len(), 2);
    assert!(matches!(
        transaction.body.statements[0],
        crate::ast::Stmt::Let { .. }
    ));
    let crate::ast::Stmt::Let { value, .. } = &statements[1] else {
        panic!(
            "expected transaction value binding, got {:?}",
            statements[1]
        );
    };
    let crate::ast::Expr::DbTransaction(value_transaction) = value else {
        panic!("expected db transaction value expression, got {value:?}");
    };
    assert_eq!(value_transaction.mode, crate::ast::DbBlockMode::Value);
    assert!(matches!(statements[2], crate::ast::Stmt::Return(_)));
}

#[test]
fn rejects_old_db_transaction_statement_block() {
    let source = r#"
            function run(id: string) -> string {
              db.transaction {}
              return id
            }
        "#;

    let error = parse_source(source).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("old db.transaction/db.* syntax is not supported"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_old_db_dotted_builtins_without_rejecting_local_receivers() {
    for operation in [
        "get",
        "require",
        "exists",
        "create",
        "createMany",
        "create_many",
        "append",
        "appendMany",
        "append_many",
        "upsert",
        "findMany",
        "find_many",
        "count",
        "transaction",
    ] {
        let source = format!(
            r#"
                function run(id: string) -> string {{
                  const old = db.{operation}
                  return id
                }}
            "#
        );
        let error = parse_source(&source).unwrap_err().to_string();
        assert!(
            error.contains("old db.transaction/db.* syntax is not supported"),
            "unexpected error for db.{operation}: {error}"
        );
    }

    let source = r#"
            function run(db: DbFactory) -> string {
              const collection = db.Collection("thread")
              return "ok"
            }
        "#;
    parse_source(source).expect("non-builtin dotted db receiver should parse as a local value");
}

#[test]
fn parses_object_db_operation_surface() {
    let source = r#"
            function run(threadId: string, rows: Array<Message>) -> string {
              const one = db require Thread(threadId) { fields { title, status } }
              const many = db find many Message { fields { id, body, where, profile.displayName; profile.avatar.url } offset 5 where threadId == threadId order createdAt desc limit 50 }
              const inserted = db insert many Message values rows
              db update Thread(threadId) { title = "Updated" messageCount += 1 }
              db delete many Message { where threadId == threadId }
              return threadId
            }
        "#;

    let ast = parse_source(source).unwrap();
    let statements = &ast.functions[0].body.statements;
    let crate::ast::Stmt::Let { value, .. } = &statements[0] else {
        panic!("expected db require binding");
    };
    let crate::ast::Expr::DbOperation(require) = value else {
        panic!("expected db operation, got {value:?}");
    };
    assert_eq!(require.op, crate::ast::DbOperationKind::Require);
    assert_eq!(require.target.name, "Thread");
    assert_eq!(require.projection.as_ref().unwrap().fields[0].text, "title");
    assert_eq!(
        require.projection.as_ref().unwrap().fields[1].text,
        "status"
    );

    let crate::ast::Stmt::Let { value, .. } = &statements[1] else {
        panic!("expected db find many binding");
    };
    let crate::ast::Expr::DbOperation(find_many) = value else {
        panic!("expected db operation, got {value:?}");
    };
    assert_eq!(find_many.op, crate::ast::DbOperationKind::Find);
    assert!(find_many.many);
    assert_eq!(
        find_many.query.as_ref().unwrap().order[0].field.text,
        "createdAt"
    );
    assert_eq!(find_many.projection.as_ref().unwrap().fields[0].text, "id");
    assert_eq!(
        find_many.projection.as_ref().unwrap().fields[2].text,
        "where"
    );
    assert_eq!(
        find_many.projection.as_ref().unwrap().fields[3].text,
        "profile.displayName"
    );
    assert_eq!(
        find_many.projection.as_ref().unwrap().fields[4].text,
        "profile.avatar.url"
    );
    assert!(matches!(
        find_many.query.as_ref().unwrap().offset.as_deref(),
        Some(crate::ast::Expr::Literal(crate::ast::Literal::Number(value))) if *value == 5.0
    ));

    let crate::ast::Stmt::Expr(crate::ast::Expr::DbOperation(update)) = &statements[3] else {
        panic!("expected update db operation");
    };
    assert_eq!(update.op, crate::ast::DbOperationKind::Update);
    assert_eq!(update.change.as_ref().unwrap().ops.len(), 2);
}

#[test]
fn rejects_old_unbounded_db_projection_syntax() {
    let source = r#"
            function run(id: string) -> string {
              const user = db require User(id) { fields name visits }
              return id
            }
        "#;

    let error = parse_source(source).unwrap_err().to_string();
    assert!(
        error.contains("db read projection now uses `fields { ... }`"),
        "unexpected old projection syntax error: {error}"
    );
}

#[test]
fn rejects_query_entries_after_key_read_projection() {
    let source = r#"
            function run(id: string, active: bool) -> string {
              const user = db require User(id) { fields { name } where active }
              return id
            }
        "#;

    let error = parse_source(source).unwrap_err().to_string();
    assert!(
        error.contains("db key reads only support fields in the following block"),
        "unexpected key read query block error: {error}"
    );
}

#[test]
fn parses_db_query_value_and_conditional_where() {
    let source = r#"
            function run(enabled: bool, owner: string) -> string {
              const query = db query Thread {
                where ownerId == owner
                where if enabled { archived == false }
                order updatedAt desc
                limit 20
              }
              return owner
            }
        "#;

    let ast = parse_source(source).expect("db query expression should parse");
    let crate::ast::Stmt::Let { value, .. } = &ast.functions[0].body.statements[0] else {
        panic!("expected db query binding");
    };
    let crate::ast::Expr::DbQuery(query) = value else {
        panic!("expected db query expression, got {value:?}");
    };
    assert_eq!(query.target.name, "Thread");
    assert_eq!(query.query.where_clauses.len(), 2);
    assert!(matches!(
        &query.query.where_clauses[0],
        crate::ast::DbWhereClause::Predicate {
            predicate: crate::ast::Expr::Binary { .. }
        }
    ));
    assert!(matches!(
        &query.query.where_clauses[1],
        crate::ast::DbWhereClause::Conditional {
            condition: crate::ast::Expr::Identifier(name),
            predicate: crate::ast::Expr::Binary { .. },
        } if name == "enabled"
    ));
    assert_eq!(query.query.order[0].field.text, "updatedAt");
}

#[test]
fn parses_dotted_db_operation_targets() {
    let source = r#"
            function run(id: string) -> string {
              const one = db require root.prompt_model.PromptDocument(id)
              const many = db find many prompt_model.PromptDocument { limit 1 }
              return one.id
            }
        "#;

    let ast = parse_source(source).expect("dotted db operation targets should parse");
    let statements = &ast.functions[0].body.statements;
    let crate::ast::Stmt::Let { value, .. } = &statements[0] else {
        panic!("expected db require binding");
    };
    let crate::ast::Expr::DbOperation(require) = value else {
        panic!("expected db require operation");
    };
    assert_eq!(require.target.name, "root.prompt_model.PromptDocument");

    let crate::ast::Stmt::Let { value, .. } = &statements[1] else {
        panic!("expected db find binding");
    };
    let crate::ast::Expr::DbOperation(find_many) = value else {
        panic!("expected db find operation");
    };
    assert_eq!(find_many.target.name, "prompt_model.PromptDocument");
}

#[test]
fn rejects_object_db_query_after_with_offset_hint() {
    let source = r#"
            function run(previous: Array<Message>) -> string {
              const many = db find many Message { after previous }
              return "ok"
            }
        "#;

    let error = parse_source(source).unwrap_err().to_string();
    assert!(
        error.contains("db query after is not supported; use offset"),
        "unexpected after query error: {error}"
    );
}

#[test]
fn rejects_many_db_operations_with_key_selectors() {
    for source in [
        r#"
            function run(id: string) -> bool {
              const row = db find many Thread(id)
              return true
            }
        "#,
        r#"
            function run(id: string) -> bool {
              db update many Thread(id) { set name = "x" }
              return true
            }
        "#,
        r#"
            function run(id: string) -> bool {
              db delete many Thread(id)
              return true
            }
        "#,
    ] {
        let error = parse_source(source).unwrap_err().to_string();
        assert!(
            error.contains("db many operations do not support key selectors"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn rejects_db_key_read_query_entries_after_selector() {
    let source = r#"
            function run(id: string) -> bool {
              const one = db require Thread(id) { where status == "open" fields { title } }
              return true
            }
        "#;

    let error = parse_source(source).unwrap_err().to_string();
    assert!(
        error.contains("db key reads only support fields"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_query_upsert_first_version() {
    let source = r#"
            function run(id: string) -> bool {
              const result = db upsert Thread { where id == id } { id = id } { title = "x" }
              return true
            }
        "#;

    let error = parse_source(source).unwrap_err().to_string();
    assert!(
        error.contains("db upsert only supports key selectors"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_test_default_run_declaration() {
    let source = r#"test defaultRun false"#;

    let ast = parse_source(source).unwrap();

    assert_eq!(ast.test_default_run, Some(false));
    assert!(ast.tests.is_empty());
}

#[test]
fn production_source_text_strips_tests_without_removing_same_line_code() {
    let expected = r#"function helper() -> number { return 1 }"#;
    let source = r#"function helper() -> number { return 1 } test defaultRun false test "same line" { assert true }"#;

    let ast = parse_source(source).unwrap();
    let production_source = crate::ast::source_text_without_test_declarations(source, &ast);

    assert_eq!(production_source, expected);
    assert!(!production_source.contains("defaultRun"));
    assert!(!production_source.contains("same line"));
}

#[test]
fn production_source_text_keeps_inline_declaration_separator() {
    let expected = r#"const first: number = 1 function second() -> number { return 2 }"#;
    let source = r#"const first: number = 1 test "inline" { assert true } function second() -> number { return 2 }"#;

    let ast = parse_source(source).unwrap();
    let production_source = crate::ast::source_text_without_test_declarations(source, &ast);

    assert_eq!(production_source, expected);
    assert!(!production_source.contains("inline"));
    assert!(!production_source.contains("assert true"));
}

#[test]
fn production_source_text_keeps_newline_after_inline_test() {
    let expected =
        "function first() -> number { return 1 }\nfunction second() -> number { return 2 }";
    let source = "function first() -> number { return 1 } test \"inline\" { assert true }\nfunction second() -> number { return 2 }";

    let ast = parse_source(source).unwrap();
    let production_source = crate::ast::source_text_without_test_declarations(source, &ast);

    assert_eq!(production_source, expected);
    assert!(!production_source.contains("inline"));
    assert!(!production_source.contains("assert true"));
}

#[test]
fn production_source_text_separates_multiline_inline_test_boundaries() {
    let expected = r#"const first: number = 1 function second() -> number { return 2 }"#;
    let source = r#"const first: number = 1 test "inline" {
  assert true
} function second() -> number { return 2 }"#;

    let ast = parse_source(source).unwrap();
    let production_source = crate::ast::source_text_without_test_declarations(source, &ast);

    assert_eq!(production_source, expected);
    assert!(!production_source.contains("inline"));
    assert!(!production_source.contains("assert true"));
}

#[test]
fn source_level_export_root_re_export_is_rejected() {
    let error = parse_source("export root.types.LlmRequest").unwrap_err();
    assert!(error.to_string().contains("expected top-level declaration"));
}

#[test]
fn source_level_export_alias_re_export_is_rejected() {
    let error = parse_source("export root.http.HttpRequest as http.Request").unwrap_err();
    assert!(error.to_string().contains("expected top-level declaration"));
}

#[test]
fn parses_any_interface_type_annotations() {
    let ast = parse_source(
        r#"
interface ToolProvider {
  function list(self: Self) -> Array<string>
}

function useProvider(
  provider: any ToolProvider,
  mapper: fn(input: any ToolProvider) -> any ToolProvider
) -> void {
  return
}
"#,
    )
    .unwrap();

    let function = &ast.functions[0];
    assert_eq!(function.params[0].ty.name, "any ToolProvider");
    assert_eq!(
        function.params[1].ty.name,
        "fn(input: any ToolProvider) -> any ToolProvider"
    );
    assert_eq!(function.return_type.name, "void");
}

#[test]
fn parses_interface_box_expression_after_construct_and_generics() {
    let ast = parse_source(
        r#"
type RepoImpl {}
interface Repository<T, U> {
  function load(self: Self, id: T) -> U
}
function make() -> void {
  const provider = RepoImpl {} as Repository<string, User>
}
"#,
    )
    .unwrap();

    let crate::ast::Stmt::Let { value, .. } = &ast.functions[0].body.statements[0] else {
        panic!("expected let statement");
    };
    let crate::ast::Expr::InterfaceBox { value, interface } = value else {
        panic!("expected interface box expression, got {value:?}");
    };
    assert_eq!(interface.name, "Repository<string, User>");
    assert!(matches!(
        value.as_ref(),
        crate::ast::Expr::Record { type_name, .. } if type_name == "RepoImpl"
    ));
}

#[test]
fn parses_remote_public_instance_source_for_direct_call_and_boxing() {
    let ast = parse_source(
        r#"
interface LlmClient {
  function send(self: Self, input: string) -> string
}
function make() -> void {
  const response = remoteLlm/managedLlm.send("hi")
  const boxed = remoteLlm/managedLlm as LlmClient
}
"#,
    )
    .unwrap();

    let crate::ast::Stmt::Let { value: call, .. } = &ast.functions[0].body.statements[0] else {
        panic!("expected call let statement");
    };
    let crate::ast::Expr::Call { callee, .. } = call else {
        panic!("expected remote direct call expression, got {call:?}");
    };
    let crate::ast::Expr::Field { object, field } = callee.as_ref() else {
        panic!("expected method field callee, got {callee:?}");
    };
    assert_eq!(field, "send");
    assert!(matches!(
        object.as_ref(),
        crate::ast::Expr::RemotePublicInstanceSource(source)
            if source.dependency_ref == "remoteLlm"
                && source.public_instance_key == "managedLlm"
    ));

    let crate::ast::Stmt::Let { value: boxed, .. } = &ast.functions[0].body.statements[1] else {
        panic!("expected boxed let statement");
    };
    assert!(matches!(
        boxed,
        crate::ast::Expr::InterfaceBox { value, interface }
            if interface.name == "LlmClient"
                && matches!(
                    value.as_ref(),
                    crate::ast::Expr::RemotePublicInstanceSource(source)
                        if source.dependency_ref == "remoteLlm"
                            && source.public_instance_key == "managedLlm"
                )
    ));
}

#[test]
fn slash_with_whitespace_remains_binary_division() {
    let ast = parse_source(
        r#"
function make(a: number, b: number) -> void {
  const value = a / b
}
"#,
    )
    .unwrap();

    let crate::ast::Stmt::Let { value, .. } = &ast.functions[0].body.statements[0] else {
        panic!("expected let statement");
    };
    assert!(matches!(
        value,
        crate::ast::Expr::Binary {
            op: crate::ast::BinaryOp::Div,
            ..
        }
    ));
}

#[test]
fn interface_box_has_higher_precedence_than_binary_expression() {
    let ast = parse_source(
        r#"
interface ToolProvider {
  function list(self: Self) -> Array<string>
}
function make() -> void {
  const provider = a + b as ToolProvider
}
"#,
    )
    .unwrap();

    let crate::ast::Stmt::Let { value, .. } = &ast.functions[0].body.statements[0] else {
        panic!("expected let statement");
    };
    let crate::ast::Expr::Binary { left, right, .. } = value else {
        panic!("expected binary expression");
    };
    assert!(matches!(
        left.as_ref(),
        crate::ast::Expr::Identifier(name) if name == "a"
    ));
    assert!(matches!(
        right.as_ref(),
        crate::ast::Expr::InterfaceBox { interface, .. } if interface.name == "ToolProvider"
    ));
}

#[test]
fn interface_box_rejects_any_interface_rhs_in_parser() {
    let error = parse_source(
        r#"
function make() -> void {
  const provider = value as any ToolProvider
}
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("use `as I`, not `as any I`"));
}

#[test]
fn export_modifier_is_rejected_in_full_mode() {
    let error = parse_source("export type Foo { a: string }").unwrap_err();
    assert!(error
        .to_string()
        .contains("the export modifier has been removed; declare public API in api.yml"));
}

#[test]
fn export_modifier_is_rejected_in_metadata_mode() {
    let error = parse_source_metadata("export type Foo { a: string }").unwrap_err();
    assert!(error
        .to_string()
        .contains("the export modifier has been removed; declare public API in api.yml"));
}
