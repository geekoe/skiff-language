use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use crate::{
    file_ir::{
        BoxSourceIr, CallIr, CallTargetIr, ExecutableIr, ExprIr, ExprRefIr, MetadataValue,
        PackageRefIr, StmtIr, TypeRefIr,
    },
    source_unit_lowering::symbol,
};
use skiff_artifact_model::{
    validate_file_ir_service_calls, ContractOperationId, ContractRequirement,
    InstructionSourceSite, LiteralIr, NamedUnionBranchIr, NominalTypeRefBaseIr, PackageCallableId,
    PackageLocalAbiIdentity, PatternIr, ReceiverCallAbi, ServiceProtocolIdentity, SlotKind,
    SyntheticInstructionSiteReason, TypeDeclIr, TypeDescriptorIr,
};
use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_source::{
    api::PublicTypeKind, build_package_from_parsed_sources,
    build_package_from_parsed_sources_with_dependency_analysis,
    parsed_sources::parse_publication_sources, prelude_registry::initialize_prelude_registry,
    source_graph::CompilerSourceFile, CompileParsedPackageSourcesInput, PackageCompilePolicy,
    PackageDependency, PublicationApiEntry, PublicationApiSpec, SourceCompilePackageFacts,
};

use super::*;

mod tail_call_structure;

const MODULE: &str = "internal.any_lowering";
const PACKAGE_ID: &str = "example.com/reader";
const PACKAGE_MODULE: &str = "pkg.reader";

fn initialize_test_prelude() {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should resolve");
    initialize_prelude_registry(
        &CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load"),
    )
    .expect("prelude registry should initialize");
}

fn any_interface_source() -> &'static str {
    r#"
          interface Provider {
            function name(self: Self) -> string
          }

          type HostProvider implements Provider {
            label: string,
          }

          impl HostProvider {
            function name() -> string {
              return self.label
            }
          }

          function make_box() -> void {
            let provider = HostProvider { label: "host" } as Provider
          }

          function call_box() -> string {
            let provider = HostProvider { label: "host" } as Provider
            return provider.name()
          }
	        "#
}

fn package_reader_source() -> &'static str {
    r#"
              type Model {
                value: string,
              }

	          interface Reader<T> {
	            function read(self: Self, fallback: T) -> T
	          }
	        "#
}

fn package_interface_box_source() -> &'static str {
    r#"
	          type Host implements pkg.Reader<string> {
	            value: string,
	          }

	          impl Host {
	            function read(fallback: string) -> string {
	              return fallback
	            }
	          }

	          function make_package_box() -> void {
	            let reader = Host { value: "host" } as pkg.Reader<string>
	          }
	        "#
}

fn any_interface_signature_source() -> &'static str {
    r#"
          interface Provider {
            function name(self: Self) -> string
          }

          function accept(provider: any Provider) -> void {
          }
        "#
}

fn package_any_interface_signature_source() -> &'static str {
    r#"
          function accept_package(reader: any pkg.Reader<string>) -> void {
          }
        "#
}

fn lowered_unit(source_text: &str) -> FileIrUnit {
    lowered_unit_result(source_text).expect("publication should lower")
}

fn lowered_unit_result(source_text: &str) -> std::result::Result<FileIrUnit, String> {
    initialize_test_prelude();
    let root = PathBuf::from("/test");
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/any_lowering.skiff"),
        MODULE.to_string(),
        false,
        false,
        source_text.to_string(),
        "internal/any_lowering.skiff",
    )
    .map_err(|error| error.to_string())?;
    let production_sources = vec![source];
    let parsed_sources =
        parse_publication_sources(&root, &production_sources).map_err(|error| error.to_string())?;
    let package_aliases = BTreeMap::new();
    let package_dependencies = Vec::<PackageDependency>::new();
    let model = build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
        parsed_sources,
        production_sources: Vec::new(),
        diagnostic_root: &root,
        publication_api: None,
        package_aliases: &package_aliases,
        package_dependencies: &package_dependencies,
        package_facts: None,
        package_artifacts: None,
        policy: PackageCompilePolicy::new("example.com/any-lowering"),
    })
    .map_err(|error| error.to_string())?;
    let lowered = crate::lower(&model).map_err(|error| error.to_string())?;
    lowered
        .file_ir_units()
        .first()
        .cloned()
        .ok_or_else(|| "one File IR unit should be emitted".to_string())
}

#[test]
fn source_declarations_lower_to_exact_mutually_exclusive_descriptors_and_branch_inputs() {
    let unit = lowered_unit(
        r#"
              type ShapeA { value: string }
              type ShapeB { value: string }
              type Box<T> { value: T }
              type PrimitiveFailure = string
              type UnionOne discriminator "kind" =
                ShapeA |
                Box<string> |
                { kind: "same", value: string } |
                "literal"
              type UnionTwo discriminator "kind" =
                ShapeB |
                { kind: "same", value: string } |
                "literal"
              alias TransparentFailure = ShapeA
              interface Marker {
                function label(self: Self) -> string
              }
            "#,
    );

    let declaration = |name: &str| {
        unit.type_table
            .iter()
            .find(|declaration| declaration.name == name)
            .unwrap_or_else(|| panic!("missing declaration `{name}`"))
    };
    let shape_a = declaration("ShapeA");
    let shape_b = declaration("ShapeB");
    assert!(matches!(
        (&shape_a.descriptor, &shape_b.descriptor),
        (
            TypeDescriptorIr::Record { fields: left },
            TypeDescriptorIr::Record { fields: right },
        ) if left == right
    ));
    assert!(matches!(
        declaration("PrimitiveFailure").descriptor,
        TypeDescriptorIr::Representation { ref representation }
            if representation == &TypeRefIr::builtin("string")
    ));
    assert!(matches!(
        declaration("TransparentFailure").descriptor,
        TypeDescriptorIr::Alias {
            target: TypeRefIr::LocalType { type_index: 0 },
        }
    ));
    assert!(matches!(
        declaration("Marker").descriptor,
        TypeDescriptorIr::Interface
    ));

    let TypeDescriptorIr::Union {
        branches: union_one,
    } = &declaration("UnionOne").descriptor
    else {
        panic!("UnionOne must lower as a named union");
    };
    assert_eq!(union_one.len(), 4);
    assert!(matches!(
        &union_one[0],
        NamedUnionBranchIr::ConcreteNominal {
            nominal_type: TypeRefIr::LocalType { type_index: 0 },
        }
    ));
    assert!(matches!(
        &union_one[1],
        NamedUnionBranchIr::ConcreteNominal {
            nominal_type: TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType { type_index: 2 },
                arguments,
            },
        } if arguments == &vec![TypeRefIr::builtin("string")]
    ));
    assert!(matches!(
        &union_one[2],
        NamedUnionBranchIr::SyntheticDiscriminator {
            payload_type: TypeRefIr::Record { fields },
            discriminator_field,
            discriminator_value,
        } if discriminator_field == "kind"
            && discriminator_value == "same"
            && fields["kind"] == TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: "same".to_string(),
                },
            }
    ));
    assert!(matches!(
        &union_one[3],
        NamedUnionBranchIr::Literal {
            value: LiteralIr::String { value },
        } if value == "literal"
    ));

    let TypeDescriptorIr::Union {
        branches: union_two,
    } = &declaration("UnionTwo").descriptor
    else {
        panic!("UnionTwo must lower as a distinct named union");
    };
    assert!(matches!(
        &union_two[0],
        NamedUnionBranchIr::ConcreteNominal {
            nominal_type: TypeRefIr::LocalType { type_index: 1 },
            ..
        }
    ));
    assert_eq!(union_one[2], union_two[1]);
    assert_eq!(union_one[3], union_two[2]);
    assert_ne!(declaration("UnionOne").name, declaration("UnionTwo").name);
}

#[test]
fn applied_nominals_flow_from_source_through_file_ir_signatures_sites_and_calls() {
    let unit = lowered_unit(
        r#"
              type Id = string
              type Box<T> { value: T }
              type Outer<A, B> { first: A, second: B }
              type Token<T> = string
              type Branch<T> { value: T }
              type Choice<T> discriminator "kind" =
                Branch<T> |
                { kind: "inline", value: T } |
                "literal"
              alias StringBox = Box<string>

              function use(
                stringBox: Box<string>,
                numberBox: Box<number>,
                nested: Outer<Box<string>, Array<Id>>,
                token: Token<string>,
                choice: Choice<string>
              ) -> Box<string> {
                let constructed = Box<string> { value: stringBox.value }
                let empty = Array.empty<Box<string>>()
                return constructed
              }

              function fail(value: Box<string>) -> void {
                throw value
              }

              function caught(value: Box<string>) -> void {
                let attempted = catch<Box<string>>(throw value)
              }

              function inspected(boxed: Box<string>) -> void {
                match boxed {
                  Box<string> { value } => {
                  }
                  _ => {
                  }
                }
              }
            "#,
    );
    assert_eq!(unit.schema_version, "skiff-file-ir-v13");
    assert_eq!(unit.ir_format_version, "skiff-file-ir-format-v7");
    assert_eq!(unit.opcode_table_version, "skiff-opcode-table-v2");

    let declaration = |name: &str| {
        unit.type_table
            .iter()
            .find(|declaration| declaration.name == name)
            .unwrap_or_else(|| panic!("missing declaration `{name}`"))
    };
    let index = |name: &str| {
        unit.type_table
            .iter()
            .position(|declaration| declaration.name == name)
            .unwrap_or_else(|| panic!("missing declaration `{name}`")) as u32
    };
    let applied = |name: &str, arguments: Vec<TypeRefIr>| TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType {
            type_index: index(name),
        },
        arguments,
    };
    let string_box = applied("Box", vec![TypeRefIr::builtin("string")]);
    let number_box = applied("Box", vec![TypeRefIr::builtin("number")]);

    assert_eq!(declaration("Box").type_params, ["T".to_string()]);
    assert!(matches!(
        &declaration("Box").descriptor,
        TypeDescriptorIr::Record { fields }
            if fields["value"] == TypeRefIr::TypeParam { name: "T".to_string() }
    ));
    assert_eq!(declaration("Token").type_params, ["T".to_string()]);
    assert!(matches!(
        declaration("Token").descriptor,
        TypeDescriptorIr::Representation { ref representation }
            if representation == &TypeRefIr::builtin("string")
    ));
    assert!(matches!(
        &declaration("StringBox").descriptor,
        TypeDescriptorIr::Alias { target } if target == &string_box
    ));

    let TypeDescriptorIr::Union {
        branches: choice_branches,
    } = &declaration("Choice").descriptor
    else {
        panic!("generic Choice must remain a named union");
    };
    assert!(matches!(
        &choice_branches[0],
        NamedUnionBranchIr::ConcreteNominal {
            nominal_type:
                TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::LocalType { type_index },
                    arguments,
                },
        } if *type_index == index("Branch")
            && arguments == &vec![TypeRefIr::TypeParam { name: "T".to_string() }]
    ));
    assert!(matches!(
        &choice_branches[1],
        NamedUnionBranchIr::SyntheticDiscriminator {
            payload_type: TypeRefIr::Record { fields },
            ..
        } if fields["value"] == TypeRefIr::TypeParam { name: "T".to_string() }
    ));
    assert!(matches!(
        &choice_branches[2],
        NamedUnionBranchIr::Literal { .. }
    ));

    let use_executable = executable(&unit, "use");
    assert_eq!(use_executable.params[0].ty, string_box);
    assert_eq!(use_executable.params[1].ty, number_box);
    assert_ne!(use_executable.params[0].ty, use_executable.params[1].ty);
    assert_eq!(
        use_executable.params[2].ty,
        applied(
            "Outer",
            vec![
                string_box.clone(),
                TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![TypeRefIr::LocalType {
                        type_index: index("Id"),
                    }],
                },
            ],
        )
    );
    assert_eq!(
        use_executable.params[3].ty,
        applied("Token", vec![TypeRefIr::builtin("string")])
    );
    assert_eq!(
        use_executable.params[4].ty,
        applied("Choice", vec![TypeRefIr::builtin("string")])
    );
    assert_eq!(use_executable.return_type, string_box);
    assert!(use_executable.body.expressions.iter().any(|expression| {
        matches!(
            expression,
            ExprIr::Construct { type_ref, .. } if type_ref == &string_box
        )
    }));
    assert!(use_executable.body.expressions.iter().any(|expression| {
        matches!(
            expression,
            ExprIr::Call { call }
                if call.type_args.get("T0") == Some(&string_box)
        )
    }));

    let failed = executable(&unit, "fail");
    assert!(failed.body.statements.iter().any(|statement| {
        matches!(
            statement,
            skiff_artifact_model::StmtIr::Throw { payload_type, .. }
                if payload_type == &string_box
        )
    }));
    let caught = executable(&unit, "caught");
    assert!(caught.body.expressions.iter().any(|expression| {
        matches!(
            expression,
            ExprIr::Throw { payload_type, .. } if payload_type == &string_box
        )
    }));
    assert!(caught.body.expressions.iter().any(|expression| {
        matches!(
            expression,
            ExprIr::Catch { catch_type, .. } if catch_type == &string_box
        )
    }));
    let inspected = executable(&unit, "inspected");
    assert!(inspected.body.statements.iter().any(|statement| {
        matches!(
            statement,
            skiff_artifact_model::StmtIr::Match { arms, .. }
                if matches!(
                    &arms[0].pattern,
                    skiff_artifact_model::PatternIr::Type { ty } if ty == &string_box
                )
        )
    }));

    let wire = serde_json::to_string(&unit).expect("File IR serializes");
    assert!(wire.contains("\"kind\":\"appliedNominal\""));
    assert!(!wire.contains("\"typeArguments\""));
}

#[test]
fn explicit_representation_constructors_preserve_wraps_order_and_throw_site() {
    let unit = lowered_unit(
        r#"
              type Plain = string
              type Generic<A, B> = string
              type Inner = string
              type Outer = Inner

              function payload(value: string) -> string {
                return value
              }

              function plain() -> Plain {
                return Plain("plain")
              }

              function generic() -> Generic<number, string> {
                return Generic<number, string>("generic")
              }

              function passthrough(value: Plain) -> Plain {
                return value
              }

              function nested() -> Outer {
                return Outer(Inner(payload("nested")))
              }

              function fail() -> void {
                throw Plain(payload("failure"))
              }
            "#,
    );
    let type_index = |name: &str| {
        unit.type_table
            .iter()
            .position(|declaration| declaration.name == name)
            .unwrap_or_else(|| panic!("missing representation `{name}`")) as u32
    };
    let plain_type = TypeRefIr::LocalType {
        type_index: type_index("Plain"),
    };
    let generic_type = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType {
            type_index: type_index("Generic"),
        },
        arguments: vec![TypeRefIr::builtin("number"), TypeRefIr::builtin("string")],
    };
    let inner_type = TypeRefIr::LocalType {
        type_index: type_index("Inner"),
    };
    let outer_type = TypeRefIr::LocalType {
        type_index: type_index("Outer"),
    };

    let only_wrap = |name: &str| {
        executable(&unit, name)
            .body
            .expressions
            .iter()
            .filter_map(|expression| match expression {
                ExprIr::RepresentationWrap { value, type_ref } => Some((*value, type_ref.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(only_wrap("plain").len(), 1);
    assert_eq!(only_wrap("plain")[0].1, plain_type);
    assert_eq!(only_wrap("generic").len(), 1);
    assert_eq!(only_wrap("generic")[0].1, generic_type);
    assert!(
        only_wrap("passthrough").is_empty(),
        "assignability must not synthesize an implicit representation wrap"
    );

    let nested = executable(&unit, "nested");
    let nested_wraps = only_wrap("nested");
    assert_eq!(
        nested_wraps
            .iter()
            .map(|(_, type_ref)| type_ref)
            .collect::<Vec<_>>(),
        vec![&inner_type, &outer_type]
    );
    let inner_wrap_index = nested
        .body
        .expressions
        .iter()
        .position(|expression| {
            matches!(
                expression,
                ExprIr::RepresentationWrap { type_ref, .. } if type_ref == &inner_type
            )
        })
        .expect("inner representation wrap");
    let outer_wrap_index = nested
        .body
        .expressions
        .iter()
        .position(|expression| {
            matches!(
                expression,
                ExprIr::RepresentationWrap { type_ref, .. } if type_ref == &outer_type
            )
        })
        .expect("outer representation wrap");
    let nested_call_index = nested
        .body
        .expressions
        .iter()
        .position(|expression| matches!(expression, ExprIr::Call { .. }))
        .expect("nested payload call");
    assert!(nested_call_index < inner_wrap_index);
    assert_eq!(
        nested_wraps[0].0.expression as usize, nested_call_index,
        "the inner wrap must reference the once-lowered payload call"
    );
    assert!(inner_wrap_index < outer_wrap_index);
    assert_eq!(
        nested_wraps[1].0.expression as usize, inner_wrap_index,
        "the outer wrap must reference the explicit inner wrap"
    );
    assert_eq!(
        nested
            .body
            .expressions
            .iter()
            .filter(|expression| matches!(expression, ExprIr::Call { .. }))
            .count(),
        1,
        "the payload side effect must lower exactly once"
    );

    let fail = executable(&unit, "fail");
    assert_eq!(
        fail.body
            .expressions
            .iter()
            .filter(|expression| matches!(expression, ExprIr::Call { .. }))
            .count(),
        1,
        "the thrown payload side effect must lower exactly once"
    );
    let fail_call_index = fail
        .body
        .expressions
        .iter()
        .position(|expression| matches!(expression, ExprIr::Call { .. }))
        .expect("throw payload call");
    let (fail_wrap_index, fail_wrap_value) = fail
        .body
        .expressions
        .iter()
        .enumerate()
        .find_map(|(index, expression)| match expression {
            ExprIr::RepresentationWrap { value, type_ref } if type_ref == &plain_type => {
                Some((index, *value))
            }
            _ => None,
        })
        .expect("direct throw representation wrap");
    assert!(fail_call_index < fail_wrap_index);
    assert_eq!(fail_wrap_value.expression as usize, fail_call_index);
    assert!(fail.body.statements.iter().any(|statement| {
        matches!(
            statement,
            skiff_artifact_model::StmtIr::Throw {
                value,
                payload_type,
                site: InstructionSourceSite::Source { span },
            } if value.expression as usize == fail_wrap_index
                && payload_type == &plain_type
                && span.source_id == 0
                && span.start.line > 0
        )
    }));
}

#[test]
fn representation_wrap_preserves_external_package_owner_in_ordered_arguments() {
    let unit = lowered_unit_with_package_facts(
        r#"
              type Generic<A, B> = string

              function make() -> Generic<pkg.Model, number> {
                return Generic<pkg.Model, number>("value")
              }
            "#,
    );
    let make = executable(&unit, "make");
    let type_ref = make
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            ExprIr::RepresentationWrap { type_ref, .. } => Some(type_ref),
            _ => None,
        })
        .expect("external package argument representation wrap");

    assert!(
        matches!(
            type_ref,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
            arguments,
        } if matches!(
            arguments.as_slice(),
            [
                TypeRefIr::PackageSymbol { symbol },
                TypeRefIr::Builtin {
                    name,
                    args,
                },
            ] if matches!(
                &symbol.package,
                PackageRefIr::Dependency { dependency_ref } if dependency_ref == "pkg"
            )
                && symbol.symbol_path == "Model"
                && symbol.abi_expectation.is_none()
                && name == "number"
                && args.is_empty()
        )
        ),
        "{type_ref:#?}"
    );
    assert!(unit.external_refs.package_symbols.iter().any(|symbol| {
        matches!(
            &symbol.package,
            PackageRefIr::Dependency { dependency_ref } if dependency_ref == "pkg"
        ) && symbol.symbol_path == "Model"
    }));
}

#[test]
fn non_representation_constructor_target_remains_a_source_error() {
    let error = lowered_unit_result(
        r#"
              type Record { value: string }

              function invalid() -> void {
                Record("not a record constructor")
              }
            "#,
    )
    .expect_err("a record call must not become a representation wrap");

    assert!(error.contains("Record"), "{error}");
    assert!(
        error.contains("unresolved")
            || error.contains("not resolved")
            || error.contains("unsupported"),
        "{error}"
    );
}

#[test]
fn source_calls_and_throws_keep_real_sites_and_catch_type_is_required() {
    let unit = lowered_unit(
        r#"
              type Failure { message: string }

              function callee(value: string) -> string {
                return value
              }

              function statement(failure: Failure) -> void {
                callee("call")
                throw failure
              }

              function expression(failure: Failure) -> Failure {
                return throw failure
              }

              function caught(value: string) -> void {
                let attempted = catch<Failure>(callee(value))
              }
            "#,
    );

    let statement = executable(&unit, "statement");
    let call = statement
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            ExprIr::Call { call } => Some(call),
            _ => None,
        })
        .expect("source call lowers");
    assert!(matches!(
        call.site,
        InstructionSourceSite::Source { ref span }
            if span.source_id == 0 && span.start.line > 0
    ));
    let throw_site = statement
        .body
        .statements
        .iter()
        .find_map(|statement| match statement {
            skiff_artifact_model::StmtIr::Throw {
                payload_type, site, ..
            } => Some((payload_type, site)),
            _ => None,
        })
        .expect("statement throw lowers");
    assert_eq!(throw_site.0, &TypeRefIr::LocalType { type_index: 0 });
    assert!(matches!(
        throw_site.1,
        InstructionSourceSite::Source { span }
            if span.source_id == 0 && span.start.line > 0
    ));

    let expression = executable(&unit, "expression");
    assert!(expression.body.expressions.iter().any(|expression| {
        matches!(
            expression,
            ExprIr::Throw {
                payload_type: TypeRefIr::LocalType { type_index: 0 },
                site: InstructionSourceSite::Source { span },
                ..
            } if span.source_id == 0 && span.start.line > 0
        )
    }));

    let caught = executable(&unit, "caught");
    assert!(caught.body.expressions.iter().any(|expression| {
        matches!(
            expression,
            ExprIr::Catch {
                catch_type: TypeRefIr::LocalType { type_index: 0 },
                ..
            }
        )
    }));
    assert!(caught.body.expressions.iter().any(|expression| {
        matches!(
            expression,
            ExprIr::Call {
                call: CallIr {
                    site: InstructionSourceSite::Source { span },
                    ..
                },
            } if span.source_id == 0 && span.start.line > 0
        )
    }));

    let wire = serde_json::to_value(&unit).expect("File IR serializes");
    assert!(
        !wire.to_string().contains("\"catchType\":null"),
        "typed catch cannot serialize an implicit catch-all"
    );
}

#[test]
fn compiler_generated_native_wrapper_uses_only_the_wrapper_synthetic_reason() {
    let mut units = lowered_units_for_package(
        "skiff.run/std",
        vec![(
            "std/wrapper_fixture.skiff",
            "std.wrapper_fixture",
            "native function passthrough(value: string) -> string",
        )],
    );
    let unit = units.pop().expect("one native wrapper File IR unit");
    let wrapper = unit
        .executables
        .iter()
        .find(|executable| executable.symbol == "std.wrapper_fixture.passthrough")
        .expect("native wrapper executable lowers");
    let call = wrapper
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            ExprIr::Call { call } => Some(call),
            _ => None,
        })
        .expect("native wrapper contains its generated native call");

    assert!(matches!(
        call.site,
        InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
        }
    ));
}

#[test]
fn validated_package_db_schema_lowers_to_typed_file_ir() {
    let unit = lowered_unit(
        r#"
                type Owner { id: string }
                type Thread { id: string, owner: Owner }
                db object Thread {
                  primary key(id)
                  unique index byOwner(owner.id desc)
                }
            "#,
    );

    let db = unit
        .declarations
        .db
        .get("Thread")
        .expect("validated package DB declaration should lower");
    assert_eq!(db.key.name, "id");
    assert!(db.fields.iter().any(|field| field.name == "owner"));
    assert_eq!(db.indexes.len(), 1);
    assert_eq!(db.indexes[0].name, "byOwner");
    assert!(db.indexes[0].unique);
    assert_eq!(db.indexes[0].fields[0].field.text, "owner.id");
    assert_eq!(
        db.indexes[0].fields[0].field.segments,
        ["owner".to_string(), "id".to_string()]
    );
    assert_eq!(
        db.indexes[0].fields[0].direction,
        skiff_artifact_model::DbIndexDirectionIr::Desc
    );
}

#[test]
fn db_contract_lowers_without_physical_collection_and_keeps_key_index_identity() {
    let unit = lowered_unit(
        r#"
                type AgentThread { id: string, status: string, updatedAt: string }
                db contract AgentThread {
                  primary key(id)
                  index byStatusUpdated(status, updatedAt desc)
                }
            "#,
    );

    let declaration = unit
        .declarations
        .db
        .get("AgentThread")
        .expect("validated db contract should lower");
    assert_eq!(
        declaration.kind,
        skiff_artifact_model::DbObjectKindIr::Contract
    );
    assert_eq!(declaration.collection_name, None);
    assert_eq!(declaration.retention, None);
    assert!(declaration.leases.is_empty());
    assert_eq!(declaration.key.name, "id");
    assert_eq!(
        declaration.key.ty,
        skiff_artifact_model::TypeRefIr::builtin("string")
    );
    assert!(declaration
        .fields
        .iter()
        .any(|field| field.name == "status"));
    assert_eq!(declaration.indexes.len(), 1);
    assert_eq!(declaration.indexes[0].name, "byStatusUpdated");
    assert_eq!(
        declaration.indexes[0].fields[1].direction,
        skiff_artifact_model::DbIndexDirectionIr::Desc
    );
}

#[test]
fn db_contract_declaration_is_not_a_physical_collection_owner() {
    let unit = lowered_unit(
        r#"
                type AgentThread { id: string, status: string }
                db contract AgentThread {
                  primary key(id)
                  index byStatus(status)
                }
            "#,
    );

    let declaration = unit
        .declarations
        .db
        .get("AgentThread")
        .expect("validated db contract should lower");
    assert_eq!(
        declaration.kind,
        skiff_artifact_model::DbObjectKindIr::Contract
    );
    assert_eq!(declaration.collection_name, None);
}

#[test]
fn db_operation_on_contract_type_compiles_and_targets_contract_identity() {
    let unit = lowered_unit(
        r#"
                type AgentThread { id: string, status: string }
                db contract AgentThread {
                  primary key(id)
                  index byStatus(status)
                }

                function readStatus(status: string) -> Array<AgentThread> {
                  return db find many AgentThread { where status == status }
                }
            "#,
    );

    let declaration = unit
        .declarations
        .db
        .get("AgentThread")
        .expect("validated db contract should lower");
    assert_eq!(
        declaration.kind,
        skiff_artifact_model::DbObjectKindIr::Contract
    );
    assert_eq!(declaration.collection_name, None);

    let executable = executable(&unit, "readStatus");
    let expression = executable
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            skiff_artifact_model::ExprIr::DbOperation { operation } => Some(operation),
            _ => None,
        })
        .expect("contract target db find must lower to a db operation");
    assert_eq!(
        expression.target.type_name,
        "internal.any_lowering.AgentThread"
    );
}

fn lowered_units(sources: Vec<(&str, &str, &str)>) -> Vec<FileIrUnit> {
    lowered_units_for_package("example.com/publication-local-refs", sources)
}

fn lowered_units_for_package(
    package_id: &str,
    sources: Vec<(&str, &str, &str)>,
) -> Vec<FileIrUnit> {
    lowered_units_result(package_id, sources).expect("publication should lower")
}

fn lowered_units_result(
    package_id: &str,
    sources: Vec<(&str, &str, &str)>,
) -> std::result::Result<Vec<FileIrUnit>, String> {
    initialize_test_prelude();
    let root = PathBuf::from("/test");
    let production_sources = sources
        .into_iter()
        .map(|(relative_path, module_path, source_text)| {
            CompilerSourceFile::parse(
                PathBuf::from(relative_path),
                module_path.to_string(),
                false,
                false,
                source_text.to_string(),
                relative_path,
            )
            .map_err(|error| error.to_string())
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let parsed_sources =
        parse_publication_sources(&root, &production_sources).map_err(|error| error.to_string())?;
    let package_aliases = BTreeMap::new();
    let package_dependencies = Vec::<PackageDependency>::new();
    let model = build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
        parsed_sources,
        production_sources: Vec::new(),
        diagnostic_root: &root,
        publication_api: None,
        package_aliases: &package_aliases,
        package_dependencies: &package_dependencies,
        package_facts: None,
        package_artifacts: None,
        policy: PackageCompilePolicy::new(package_id),
    })
    .map_err(|error| error.to_string())?;
    crate::lower(&model)
        .map_err(|error| error.to_string())
        .map(|lowered| lowered.file_ir_units().to_vec())
}

fn lowered_unit_with_package_facts(source_text: &str) -> FileIrUnit {
    initialize_test_prelude();
    let package_root = PathBuf::from("/package");
    let package_source = CompilerSourceFile::parse(
        PathBuf::from("pkg/reader.skiff"),
        PACKAGE_MODULE.to_string(),
        false,
        false,
        package_reader_source().to_string(),
        "pkg/reader.skiff",
    )
    .expect("package source should parse");
    let package_api = PublicationApiSpec::from_entries(vec![
        PublicationApiEntry::for_source("Reader", PACKAGE_MODULE, "Reader"),
        PublicationApiEntry::for_source("Model", PACKAGE_MODULE, "Model"),
    ]);
    let package_production_sources = vec![package_source];
    let package_parsed_sources =
        parse_publication_sources(&package_root, &package_production_sources)
            .expect("package source facts should build");
    let package_aliases = BTreeMap::new();
    let package_dependencies = Vec::<PackageDependency>::new();
    let package_model = build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
        parsed_sources: package_parsed_sources,
        production_sources: package_production_sources,
        diagnostic_root: &package_root,
        publication_api: Some(&package_api),
        package_aliases: &package_aliases,
        package_dependencies: &package_dependencies,
        package_facts: None,
        package_artifacts: None,
        policy: PackageCompilePolicy::new(PACKAGE_ID),
    })
    .expect("package source model should build");
    assert_eq!(
        package_model
            .export_bindings()
            .public_schema_types()
            .get("Reader")
            .expect("Reader should be exported")
            .kind,
        PublicTypeKind::Interface
    );
    let package_lowered = crate::lower(&package_model).expect("package should lower");
    let package_file_ir_units = package_lowered.file_ir_units().to_vec();
    let package_facts = vec![SourceCompilePackageFacts::new(
        PACKAGE_ID,
        "1.0.0",
        Vec::new(),
        &package_model,
        &package_file_ir_units,
    )];

    let root = PathBuf::from("/test");
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/any_lowering.skiff"),
        MODULE.to_string(),
        false,
        false,
        source_text.to_string(),
        "internal/any_lowering.skiff",
    )
    .expect("test source should parse");
    let production_sources = vec![source];
    let parsed_sources = parse_publication_sources(&root, &production_sources)
        .expect("test source facts should build");
    let package_aliases = BTreeMap::from([("pkg".to_string(), vec![String::new()])]);
    let mut dependency = PackageDependency::id(PACKAGE_ID);
    dependency.alias = Some("pkg".to_string());
    let package_dependencies = vec![dependency];
    let model = build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
        parsed_sources,
        production_sources: Vec::new(),
        diagnostic_root: &root,
        publication_api: None,
        package_aliases: &package_aliases,
        package_dependencies: &package_dependencies,
        package_facts: Some(&package_facts),
        package_artifacts: None,
        policy: PackageCompilePolicy::new("example.com/any-lowering"),
    })
    .expect("source model with package facts should build");
    let lowered = crate::lower(&model).expect("publication should lower");
    lowered
        .file_ir_units()
        .first()
        .expect("one file IR unit should be emitted")
        .clone()
}

fn executable<'a>(unit: &'a FileIrUnit, name: &str) -> &'a ExecutableIr {
    let expected_symbol = symbol(MODULE, name);
    unit.executables
        .iter()
        .find(|executable| executable.symbol == expected_symbol)
        .unwrap_or_else(|| panic!("missing executable `{expected_symbol}`"))
}

#[test]
fn ternary_lowers_to_lazy_value_block_with_if_and_temp_slot() {
    let unit = lowered_unit(
        r#"
          function pick(flag: bool, a: string, b: string) -> string {
            return flag ? a : b
          }
        "#,
    );
    let executable = executable(&unit, "pick");
    let value_blocks = executable
        .body
        .expressions
        .iter()
        .filter_map(|expr| match expr {
            ExprIr::ValueBlock { block, result } => Some((block, *result)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        value_blocks.len(),
        1,
        "expected one ValueBlock for the ternary"
    );
    let (body_label, result_ref) = value_blocks[0];

    let ExprIr::LoadSlot { slot: result_slot } =
        &executable.body.expressions[result_ref.expression as usize]
    else {
        panic!("expected ternary result to load the temp slot");
    };

    let body_block = executable
        .body
        .blocks
        .iter()
        .find(|block| &block.label == body_label)
        .expect("ternary body block should be emitted");
    assert_eq!(body_block.statements.len(), 2);
    let skiff_artifact_model::StmtIr::Let {
        slot: init_slot,
        value: init_value,
    } = &executable.body.statements[body_block.statements[0].statement as usize]
    else {
        panic!("expected Let statement initializing the ternary temp slot");
    };
    assert_eq!(init_slot, result_slot);
    assert!(matches!(
        &executable.body.expressions[init_value.expression as usize],
        ExprIr::Literal { .. }
    ));

    let skiff_artifact_model::StmtIr::If {
        condition,
        then_block,
        else_block,
    } = &executable.body.statements[body_block.statements[1].statement as usize]
    else {
        panic!("expected If statement inside the ternary body block");
    };
    assert!(matches!(
        &executable.body.expressions[condition.expression as usize],
        ExprIr::LoadSlot { .. }
    ));

    let else_block = else_block
        .as_ref()
        .expect("ternary requires an else branch");
    for (label, expected_branch) in [(then_block, "then"), (else_block, "else")] {
        let branch_block = executable
            .body
            .blocks
            .iter()
            .find(|block| &block.label == label)
            .unwrap_or_else(|| panic!("missing ternary {expected_branch} block"));
        assert_eq!(branch_block.statements.len(), 1);
        let skiff_artifact_model::StmtIr::Assign { target, value } =
            &executable.body.statements[branch_block.statements[0].statement as usize]
        else {
            panic!("expected Assign in ternary {expected_branch} block");
        };
        assert!(matches!(
            target,
            skiff_artifact_model::AssignTargetIr::Slot { slot }
                if slot == result_slot
        ));
        assert!(matches!(
            &executable.body.expressions[value.expression as usize],
            ExprIr::LoadSlot { .. }
        ));
    }
}

fn only_interface_box(executable: &ExecutableIr) -> &ExprIr {
    let boxes = executable
        .body
        .expressions
        .iter()
        .filter(|expr| matches!(expr, ExprIr::InterfaceBox { .. }))
        .collect::<Vec<_>>();
    assert_eq!(
        boxes.len(),
        1,
        "expected exactly one InterfaceBox in {}",
        executable.symbol
    );
    boxes[0]
}

#[test]
fn emits_actor_declaration_and_exact_registry_type_arguments() {
    initialize_test_prelude();
    let unit = lowered_unit(
        r#"
              type UserActor {
                id: string,
                displayName: string,
                loginCount: number,
              }

              actor UserActor {
                key(id)
                create(displayName: string, loginCount: number)
              }

              impl UserActor {
                function create(self: UserActor, displayName: string, loginCount: number) -> void {
                  self.displayName = displayName
                  self.loginCount = loginCount
                }

                function rename(self: UserActor, value: string) -> string {
                  self.displayName = value
                  return self.displayName
                }

                function increment(delta: number) -> number {
                  self.loginCount = self.loginCount + delta
                  return self.loginCount
                }
              }

              function load(id: string) -> UserActor {
                return std.actor.get<UserActor>(id, "Ada", 1)
              }

              function invoke(actor: UserActor) -> string {
                return actor.rename("Grace")
              }
            "#,
    );

    let declaration = unit
        .actor_declarations
        .first()
        .expect("actor declaration should be emitted in its owner file");
    assert_eq!(declaration.abi.actor_name, "UserActor");
    assert_eq!(declaration.abi.actor_id_type, TypeRefIr::builtin("string"));
    assert_eq!(declaration.abi.key_field, "id");
    assert_eq!(
        declaration
            .abi
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "displayName", "loginCount"]
    );
    let create = declaration.abi.create.as_ref().expect("create signature");
    assert_eq!(create.parameters.len(), 2);
    assert_eq!(create.parameters[0].name, "displayName");
    assert_eq!(create.parameters[1].name, "loginCount");
    assert_eq!(declaration.abi.public_methods.len(), 2);
    assert!(declaration
        .abi
        .public_methods
        .iter()
        .all(|method| method.name != "create"));
    assert!(declaration.create_implementation.is_some());
    let rename = declaration
        .abi
        .public_methods
        .iter()
        .find(|method| method.name == "rename")
        .unwrap();
    assert_eq!(rename.name, "rename");
    assert_eq!(rename.parameters.len(), 1);
    assert_eq!(rename.parameters[0].name, "value");
    assert_eq!(rename.return_type, TypeRefIr::builtin("string"));
    assert!(!rename.may_suspend);
    let increment = declaration
        .abi
        .public_methods
        .iter()
        .find(|method| method.name == "increment")
        .unwrap();
    assert_eq!(
        increment
            .parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["delta"]
    );
    assert_eq!(increment.return_type, TypeRefIr::builtin("number"));
    assert_eq!(
        declaration
            .method_implementations
            .get(&rename.method_identity),
        unit.declarations
            .executables
            .get("UserActor.rename")
            .map(|entry| &entry.executable_index)
    );
    assert!(!declaration
        .actor_implementation_identity
        .as_str()
        .contains("pending"));

    let rename_executable = executable(&unit, "UserActor.rename");
    assert!(rename_executable.body.expressions.iter().any(|expression| {
        matches!(
            expression,
            ExprIr::ActorSelfField { field, field_type }
                if field == "displayName" && field_type == &TypeRefIr::builtin("string")
        )
    }));
    assert!(rename_executable.body.statements.iter().any(|statement| {
        matches!(
            statement,
            skiff_artifact_model::StmtIr::Assign {
                target: skiff_artifact_model::AssignTargetIr::ActorSelfField {
                    field,
                    field_type,
                },
                ..
            } if field == "displayName" && field_type == &TypeRefIr::builtin("string")
        )
    }));

    let load = executable(&unit, "load");
    let calls = load
            .body
            .expressions
            .iter()
            .filter_map(|expression| match expression {
                ExprIr::Call { call } => Some(call),
                _ => None,
            })
            .filter(|call| {
                matches!(
                    &call.target,
                    CallTargetIr::Native { target }
                        if target.binding_key.as_deref().is_some_and(|key| key.starts_with("std.actor."))
                )
            })
            .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    let invoke = executable(&unit, "invoke");
    let actor_call = invoke
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            ExprIr::Call {
                call:
                    call @ skiff_artifact_model::CallIr {
                        target: CallTargetIr::ActorMethod { .. },
                        ..
                    },
            } => Some(call),
            _ => None,
        })
        .expect("Actor receiver call should keep its dedicated target");
    let CallTargetIr::ActorMethod {
        actor,
        actor_abi_identity,
        actor_implementation_identity,
        method_identity,
    } = &actor_call.target
    else {
        unreachable!("find_map only returns ActorMethod");
    };
    assert_eq!(actor.module_path, MODULE);
    assert_eq!(actor.symbol, "UserActor");
    assert_eq!(actor_abi_identity, &declaration.actor_abi_identity);
    assert_eq!(
        actor_implementation_identity,
        &declaration.actor_implementation_identity
    );
    assert_eq!(method_identity, &rename.method_identity);

    let get = calls
        .iter()
        .find(|call| {
            matches!(
                &call.target,
                CallTargetIr::Native { target }
                    if target.binding_key.as_deref() == Some("std.actor.get")
            )
        })
        .expect("get call should be lowered");
    let TypeRefIr::ServiceSymbol { symbol: get_symbol } = &get.type_args["T0"] else {
        panic!("get T0 must be a service symbol: {:?}", get.type_args["T0"]);
    };
    assert_eq!(get_symbol.symbol, "UserActor");
    assert_eq!(get_symbol.module_path, MODULE);
    assert_eq!(get.type_args["T1"], TypeRefIr::builtin("string"));
    assert!(!get.type_args.contains_key("T2"));
}

#[test]
fn create_empty_body_rejects_unassigned_fields_on_implicit_end() {
    let error = lowered_unit_result(
        r#"
              type Counter {
                id: string,
                count: number,
              }

              actor Counter {
                key(id)
                create()
              }

              impl Counter {
                function create(self: Counter) -> void {
                }
              }
            "#,
    )
    .expect_err("empty create body must not fall through with unassigned fields")
    .to_string();
    assert!(
        error.contains("create returns before assigning field(s): count"),
        "unexpected create error: {error}"
    );
}

#[test]
fn create_conditional_assignment_rejects_unassigned_fields_on_implicit_end() {
    let error = lowered_unit_result(
        r#"
              type Counter {
                id: string,
                count: number,
              }

              actor Counter {
                key(id)
                create(flag: bool)
              }

              impl Counter {
                function create(self: Counter, flag: bool) -> void {
                  if flag {
                    self.count = 0
                  }
                }
              }
            "#,
    )
    .expect_err("conditional assignment must not count as definite on implicit fallthrough")
    .to_string();
    assert!(
        error.contains("create returns before assigning field(s): count"),
        "unexpected create error: {error}"
    );
}

#[test]
fn create_implicit_end_passes_when_all_fields_assigned() {
    let unit = lowered_unit_result(
        r#"
              type Counter {
                id: string,
                count: number,
              }

              actor Counter {
                key(id)
                create()
              }

              impl Counter {
                function create(self: Counter) -> void {
                  self.count = 0
                }
              }
            "#,
    )
    .expect("fully assigned create body should lower");
    assert!(unit.actor_declarations[0].create_implementation.is_some());
}

#[test]
fn create_explicit_return_before_assignment_stays_rejected() {
    let error = lowered_unit_result(
        r#"
              type Counter {
                id: string,
                count: number,
              }

              actor Counter {
                key(id)
                create()
              }

              impl Counter {
                function create(self: Counter) -> void {
                  return
                }
              }
            "#,
    )
    .expect_err("explicit return before assignment must stay rejected")
    .to_string();
    assert!(
        error.contains("create returns before assigning field(s): count"),
        "unexpected create error: {error}"
    );
}

#[test]
fn actor_create_self_method_dispatch_stays_rejected() {
    let error = lowered_unit_result(
        r#"
              type Counter {
                id: string,
                count: number,
              }

              actor Counter {
                key(id)
                create()
              }

              impl Counter {
                function create(self: Counter) -> void {
                  self.count = 0
                  let current = self
                  current.increment()
                }

                function increment(self: Counter) -> void {
                  self.count = self.count + 1
                  return
                }
              }
            "#,
    )
    .expect_err("create must not synchronously dispatch another method on itself")
    .to_string();
    assert!(
        error.contains("actor Counter create cannot call other methods of the same instance"),
        "unexpected create self-call error: {error}"
    );
}

#[test]
fn dispatch_expression_lowers_task_submit_plan_with_timing() {
    let unit = lowered_unit(
        r#"
            type Instant = Date

            function run(input: string) -> void {
              return
            }

            function start(input: string, instant: Instant) -> void {
              let afterRef = dispatch run(input) after(200ms)
              let atRef = dispatch run(input) at(instant)
              dispatch run(input)
            }
        "#,
    );
    let start = unit
        .executables
        .iter()
        .find(|executable| executable.symbol == format!("{MODULE}.start"))
        .expect("start executable");

    let StmtIr::Let {
        value: after_call_ref,
        ..
    } = &start.body.statements[0]
    else {
        panic!("first dispatch must lower into a let statement");
    };
    let after_call = task_submit_call(start, *after_call_ref);
    assert_eq!(
        after_call.args.len(),
        1,
        "dispatch payload args must evaluate each argument exactly once"
    );
    let after_timing = task_submit_timing(after_call);
    assert_eq!(
        after_timing.get("kind"),
        Some(&MetadataValue::String("after".to_string()))
    );
    let MetadataValue::Number(expr_index) = after_timing.get("expr").expect("after expr index")
    else {
        panic!("after timing must carry an expression index");
    };
    let timing_expr = &start.body.expressions[expr_index.as_u64().unwrap() as usize];
    let ExprIr::Call { call: timing_call } = timing_expr else {
        panic!("after timing must lower to a call expression");
    };
    // `after(200ms)` desugars to `Duration.milliseconds(200)`.
    assert!(
        matches!(timing_call.target, CallTargetIr::Native { .. }),
        "unexpected timing call target {:?}",
        timing_call.target
    );
    assert_eq!(timing_call.args.len(), 1);
    let ExprIr::Literal {
        value: LiteralIr::Number {
            value: milliseconds,
        },
    } = &start.body.expressions[timing_call.args[0].expression as usize]
    else {
        panic!("timing literal must lower to a number");
    };
    assert_eq!(milliseconds.as_f64(), Some(200.0));
    assert!(
        !after_call
            .args
            .iter()
            .any(|arg| arg.expression == expr_index.as_u64().unwrap() as u32),
        "timing expression must not be duplicated into the payload args"
    );

    let StmtIr::Let {
        value: at_call_ref, ..
    } = &start.body.statements[1]
    else {
        panic!("second dispatch must lower into a let statement");
    };
    let at_call = task_submit_call(start, *at_call_ref);
    assert_eq!(at_call.args.len(), 1);
    let at_timing = task_submit_timing(at_call);
    assert_eq!(
        at_timing.get("kind"),
        Some(&MetadataValue::String("at".to_string()))
    );
    let MetadataValue::Number(expr_index) = at_timing.get("expr").expect("at expr index") else {
        panic!("at timing must carry an expression index");
    };
    let ExprIr::LoadSlot { slot } = &start.body.expressions[expr_index.as_u64().unwrap() as usize]
    else {
        panic!("at timing must lower to the instant slot load");
    };
    assert!(
        matches!(slot, 1),
        "instant should occupy slot 1, got {slot}"
    );

    let StmtIr::Dispatch { call: call_ref } = &start.body.statements[2] else {
        panic!("statement dispatch must keep StmtIr::Dispatch");
    };
    let immediate_call = task_submit_call(start, *call_ref);
    let immediate_timing = task_submit_timing(immediate_call);
    assert_eq!(
        immediate_timing.get("kind"),
        Some(&MetadataValue::String("immediate".to_string()))
    );
    assert!(
        immediate_timing.get("expr").is_none(),
        "immediate timing must not carry an expression index"
    );
}

fn task_submit_call(executable: &ExecutableIr, call_ref: ExprRefIr) -> &CallIr {
    let ExprIr::Call { call } = &executable.body.expressions[call_ref.expression as usize] else {
        panic!("dispatch must reference a call expression");
    };
    call
}

fn task_submit_timing(call: &CallIr) -> &BTreeMap<String, MetadataValue> {
    let MetadataValue::Object(metadata) = call
        .metadata
        .get("dispatchSubmit")
        .expect("dispatch call must carry dispatchSubmit metadata")
    else {
        panic!("dispatchSubmit metadata must be an object");
    };
    let MetadataValue::Object(timing) = metadata.get("timing").expect("timing plan") else {
        panic!("timing plan must be an object");
    };
    timing
}

#[test]
fn actor_create_dispatch_self_method_stays_rejected() {
    let error = lowered_unit_result(
        r#"
              type Counter {
                id: string,
                count: number,
              }

              actor Counter {
                key(id)
                create()
              }

              impl Counter {
                function create(self: Counter) -> void {
                  self.count = 0
                  let current = self
                  dispatch current.increment()
                }

                function increment(self: Counter) -> void {
                  self.count = self.count + 1
                  return
                }
              }
            "#,
    )
    .expect_err("create dispatch self must stay rejected")
    .to_string();
    assert!(
        error.contains("actor Counter create cannot call other methods of the same instance"),
        "unexpected create self-dispatch error: {error}"
    );
}

#[test]
fn actor_transaction_bodies_cannot_write_actor_fields() {
    let error = lowered_unit_result(
        r#"
              type Counter {
                id: string,
                count: number,
              }

              actor Counter {
                key(id)
                create()
              }

              impl Counter {
                function create(self: Counter) -> void {
                  self.count = 0
                }

                function run(self: Counter) -> void {
                  db transaction {
                    self.count = 1
                  }
                }
              }
            "#,
    )
    .expect_err("actor transaction bodies must not write actor fields")
    .to_string();
    assert!(
        error.contains("db transaction bodies cannot write actor field count in v1"),
        "unexpected transaction body field write error: {error}"
    );

    let error = lowered_unit_result(
        r#"
              type Counter {
                id: string,
                count: number,
                items: Array<number>,
              }

              actor Counter {
                key(id)
                create()
              }

              impl Counter {
                function create(self: Counter) -> void {
                  self.count = 0
                  self.items = Array.empty<number>()
                }

                function run(self: Counter) -> void {
                  db transaction {
                    self.items.push(1)
                  }
                }
              }
            "#,
    )
    .expect_err("actor transaction bodies must not mutate actor fields")
    .to_string();
    assert!(
        error.contains("db transaction bodies cannot mutate actor fields"),
        "unexpected transaction body field mutation error: {error}"
    );
}

#[test]
fn actor_self_field_access_keeps_while_body_call_targets_aligned() {
    let units = lowered_units(vec![
        (
            "internal/worker.skiff",
            "internal.worker",
            r#"
                  function drainStopped() -> boolean {
                    return false
                  }
                "#,
        ),
        (
            "internal/runner.skiff",
            "internal.runner",
            r#"
                  type UserActor {
                    id: string,
                    displayName: string,
                  }

                  actor UserActor {
                    key(id)
                    create(displayName: string)
                  }

                  function isStopped() -> boolean {
                    return false
                  }

                  impl UserActor {
                    function create(self: UserActor, displayName: string) -> void {
                      self.displayName = displayName
                    }

                    function run(self: UserActor, other: UserActor) -> void {
                      let name = self.displayName
                      self.displayName = name
                      while root.internal.runner.isStopped() {
                        self.displayName = other.rename(name)
                        if root.internal.worker.drainStopped() {
                          self.run(other)
                          break
                        }
                      }
                    }

                    function rename(self: UserActor, value: string) -> string {
                      return value
                    }
                  }
                "#,
        ),
    ]);
    let worker = units
        .iter()
        .find(|unit| unit.module_path == "internal.worker")
        .expect("worker unit should be emitted");
    let runner = units
        .iter()
        .find(|unit| unit.module_path == "internal.runner")
        .expect("runner unit should be emitted");
    let is_stopped_index = runner.declarations.executables["isStopped"].executable_index;
    let drain_index = worker.declarations.executables["drainStopped"].executable_index;
    let run = runner
        .executables
        .iter()
        .find(|executable| executable.symbol == "internal.runner.UserActor.run")
        .expect("actor run executable should exist");

    assert!(run.body.expressions.iter().any(|expression| matches!(
        expression,
        ExprIr::ActorSelfField { field, .. } if field == "displayName"
    )));
    assert!(run.body.statements.iter().any(|statement| matches!(
        statement,
        skiff_artifact_model::StmtIr::Assign {
            target: skiff_artifact_model::AssignTargetIr::ActorSelfField { field, .. },
            ..
        } if field == "displayName"
    )));
    let calls = run
        .body
        .expressions
        .iter()
        .filter_map(|expression| match expression {
            ExprIr::Call { call } => Some(call),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        calls.iter().any(|call| matches!(
            &call.target,
            CallTargetIr::LocalExecutable { executable_index }
                if *executable_index == is_stopped_index
        )),
        "same-module root call in while condition must stay LocalExecutable: {calls:#?}"
    );
    assert!(
        calls.iter().any(|call| matches!(
            &call.target,
            CallTargetIr::PublicationExecutable {
                module_path,
                executable_index,
            } if module_path == "internal.worker" && *executable_index == drain_index
        )),
        "cross-module root call in while body must stay PublicationExecutable: {calls:#?}"
    );
    let actor_methods = calls
        .iter()
        .filter(|call| {
            matches!(
                &call.target,
                CallTargetIr::ActorMethod {
                    actor,
                    ..
                } if actor.module_path == "internal.runner" && actor.symbol == "UserActor"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actor_methods.len(),
        2,
        "self.run and actor handle rename must both lower to ActorMethod: {calls:#?}"
    );
    assert!(
        calls
            .iter()
            .all(|call| !matches!(&call.target, CallTargetIr::Builtin { .. })),
        "calls after self field access in a while body must not fall back to Builtin: {calls:#?}"
    );
}

#[test]
fn lowers_cross_module_publication_refs_to_direct_addresses() {
    let units = lowered_units(vec![
        (
            "internal/worker.skiff",
            "internal.worker",
            r#"
                  type DrainResult {
                    value: string,
                  }

                  function drain() -> DrainResult {
                    return DrainResult { value: "ok" }
                  }
                "#,
        ),
        (
            "internal/runner.skiff",
            "internal.runner",
            r#"
                  function run() -> root.internal.worker.DrainResult {
                    return root.internal.worker.drain()
                  }
                "#,
        ),
    ]);
    let worker = units
        .iter()
        .find(|unit| unit.module_path == "internal.worker")
        .expect("worker unit should be emitted");
    let runner = units
        .iter()
        .find(|unit| unit.module_path == "internal.runner")
        .expect("runner unit should be emitted");
    let result_type_index = worker
        .declarations
        .types
        .get("DrainResult")
        .expect("DrainResult declaration should exist")
        .type_index;
    let drain_executable_index = worker
        .declarations
        .executables
        .get("drain")
        .expect("drain declaration should exist")
        .executable_index;
    let run = runner
        .executables
        .iter()
        .find(|executable| executable.symbol == "internal.runner.run")
        .expect("run executable should exist");

    assert!(matches!(
        &run.return_type,
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } if module_path == "internal.worker" && *type_index == result_type_index
    ));
    assert!(
        run.body.expressions.iter().any(|expr| matches!(
            expr,
            ExprIr::Call {
                call
            } if matches!(
                &call.target,
                CallTargetIr::PublicationExecutable {
                    module_path,
                    executable_index,
                } if module_path == "internal.worker"
                    && *executable_index == drain_executable_index
            )
        )),
        "cross-module function call should lower to PublicationExecutable"
    );
    assert!(
        runner.external_refs.service_symbols.is_empty(),
        "publication-local refs must not remain in external_refs: {:?}",
        runner.external_refs.service_symbols
    );
    assert!(runner.link_targets.types.is_empty());
    assert!(runner.link_targets.executables.is_empty());
    assert!(worker.link_targets.types.is_empty());
    assert!(worker.link_targets.executables.is_empty());
}

#[test]
fn lowers_cross_module_generic_function_to_exact_publication_executable() {
    let units = lowered_units(vec![
        (
            "internal/worker.skiff",
            "internal.worker",
            r#"
                  function identity<T>(value: T) -> T {
                    return value
                  }
                "#,
        ),
        (
            "internal/runner.skiff",
            "internal.runner",
            r#"
                  function run() -> string {
                    return root.internal.worker.identity<string>("ok")
                  }
                "#,
        ),
    ]);
    let worker = units
        .iter()
        .find(|unit| unit.module_path == "internal.worker")
        .unwrap();
    let runner = units
        .iter()
        .find(|unit| unit.module_path == "internal.runner")
        .unwrap();
    let expected_index = worker.declarations.executables["identity"].executable_index;
    let call = runner
        .executables
        .iter()
        .find(|executable| executable.symbol == "internal.runner.run")
        .expect("runner executable")
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            ExprIr::Call { call } => Some(call),
            _ => None,
        })
        .expect("generic call should lower");
    assert!(matches!(
        call.target,
        CallTargetIr::PublicationExecutable {
            ref module_path,
            executable_index,
        } if module_path == "internal.worker" && executable_index == expected_index
    ));
    assert_eq!(call.type_args["T0"], TypeRefIr::builtin("string"));
}

#[test]
fn lowers_cross_module_generic_impl_receiver_to_exact_publication_executable() {
    let units = lowered_units(vec![
        (
            "internal/worker.skiff",
            "internal.worker",
            r#"
                  type Box<T> { value: T }

                  impl Box<T> {
                    function unwrap() -> T {
                      return self.value
                    }
                  }
                "#,
        ),
        (
            "internal/runner.skiff",
            "internal.runner",
            r#"
                  function run(box: root.internal.worker.Box<string>) -> string {
                    return box.unwrap()
                  }
                "#,
        ),
    ]);
    let worker = units
        .iter()
        .find(|unit| unit.module_path == "internal.worker")
        .unwrap();
    let runner = units
        .iter()
        .find(|unit| unit.module_path == "internal.runner")
        .unwrap();
    let expected_index = worker.declarations.executables["Box<T>.unwrap"].executable_index;
    let call = runner
        .executables
        .iter()
        .find(|executable| executable.symbol == "internal.runner.run")
        .expect("runner executable")
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            ExprIr::Call { call } => Some(call),
            _ => None,
        })
        .expect("generic receiver call should lower");
    assert!(matches!(
        call.target,
        CallTargetIr::PublicationExecutable {
            ref module_path,
            executable_index,
        } if module_path == "internal.worker" && executable_index == expected_index
    ));
    assert_eq!(call.type_args["T0"], TypeRefIr::builtin("string"));
}

#[test]
fn lowers_cross_module_const_initializer_call_to_exact_publication_executable() {
    let units = lowered_units(vec![
        (
            "internal/worker.skiff",
            "internal.worker",
            r#"
                  function label() -> string {
                    return "worker"
                  }
                "#,
        ),
        (
            "internal/runner.skiff",
            "internal.runner",
            r#"
                  const LABEL: string = root.internal.worker.label()

                  function run() -> string {
                    return LABEL
                  }
                "#,
        ),
    ]);
    let worker = units
        .iter()
        .find(|unit| unit.module_path == "internal.worker")
        .unwrap();
    let runner = units
        .iter()
        .find(|unit| unit.module_path == "internal.runner")
        .unwrap();
    let expected_index = worker.declarations.executables["label"].executable_index;
    let call = runner.constants[0]
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            ExprIr::Call { call } => Some(call),
            _ => None,
        })
        .expect("const initializer call should lower");
    assert!(matches!(
        call.target,
        CallTargetIr::PublicationExecutable {
            ref module_path,
            executable_index,
        } if module_path == "internal.worker" && executable_index == expected_index
    ));
}

#[test]
fn rethrow_statement_keeps_following_local_recursive_call_target_aligned() {
    let unit = lowered_unit(
        r#"
              function attempt() -> void {
                return null
              }

              function retry(remainingAttempts: integer) -> void {
                let result = catch<std.db.ConflictError>(attempt())
                if result.tag == "ok" {
                  return null
                }
                if remainingAttempts == 0 {
                  let exception = result.exception
                  rethrow exception
                }
                return retry(remainingAttempts)
              }
            "#,
    );
    let retry_index = unit.declarations.executables["retry"].executable_index;
    let retry = unit
        .executables
        .iter()
        .find(|executable| executable.symbol == format!("{MODULE}.retry"))
        .expect("retry executable");

    assert!(
        retry.body.expressions.iter().any(|expression| matches!(
            expression,
            ExprIr::Call { call }
                if matches!(
                    call.target,
                    CallTargetIr::LocalExecutable { executable_index }
                        if executable_index == retry_index
                )
        )),
        "the self-recursive call after rethrow must retain its exact local target"
    );
}

#[test]
fn generic_function_and_impl_self_recursion_use_exact_local_targets() {
    let unit = lowered_unit(
        r#"
              type Box<T> {
                value: T,
              }

              function retryValue<T>(value: T, remainingAttempts: integer) -> T {
                if remainingAttempts == 0 {
                  return value
                }
                return retryValue<T>(value, remainingAttempts)
              }

              impl Box<T> {
                function retry(remainingAttempts: integer) -> T {
                  if remainingAttempts == 0 {
                    return self.value
                  }
                  return self.retry(remainingAttempts)
                }
              }
            "#,
    );
    for declaration_name in ["retryValue", "Box<T>.retry"] {
        let expected_index = unit.declarations.executables[declaration_name].executable_index;
        let executable = &unit.executables[expected_index as usize];
        assert!(
            executable
                .body
                .expressions
                .iter()
                .any(|expression| matches!(
                    expression,
                    ExprIr::Call { call }
                        if matches!(
                            call.target,
                            CallTargetIr::LocalExecutable { executable_index }
                                if executable_index == expected_index
                        )
                )),
            "`{declaration_name}` must resolve its self-edge to its canonical executable index"
        );
    }
}

#[test]
fn ambiguous_generic_impl_receiver_fails_before_file_ir() {
    let error = lowered_units_result(
        "example.com/ambiguous-generic-impl",
        vec![
            (
                "internal/worker.skiff",
                "internal.worker",
                r#"
                      type Box<T> { value: T }

                      impl Box<T> {
                        function unwrap() -> T { return self.value }
                      }

                      impl Box<U> {
                        function unwrap() -> U { return self.value }
                      }
                    "#,
            ),
            (
                "internal/runner.skiff",
                "internal.runner",
                r#"
                      function run(box: root.internal.worker.Box<string>) -> string {
                        return box.unwrap()
                      }
                    "#,
            ),
        ],
    )
    .expect_err("ambiguous generic impl receiver must fail closed");
    assert!(
        error.contains("duplicate")
            || error.contains("ambiguous")
            || error.contains("more than once")
            || error.contains("no exact typed source target"),
        "unexpected ambiguity diagnostic: {error}"
    );
}

#[test]
fn lowers_current_package_symbol_types_to_direct_publication_addresses() {
    let units = lowered_units_for_package(
        skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID,
        vec![
            (
                "std/time.skiff",
                "std.time",
                r#"
                      type Duration = integer

                      function identity(duration: Duration) -> Duration {
                        return duration
                      }
                    "#,
            ),
            (
                "std/consumer.skiff",
                "std.consumer",
                r#"
                      function passthrough(duration: std.time.Duration) -> std.time.Duration {
                        return duration
                      }
                    "#,
            ),
        ],
    );
    let time = units
        .iter()
        .find(|unit| unit.module_path == "std.time")
        .expect("std.time unit should be emitted");
    let consumer = units
        .iter()
        .find(|unit| unit.module_path == "std.consumer")
        .expect("std.consumer unit should be emitted");
    let duration_type_index = time
        .declarations
        .types
        .get("Duration")
        .expect("Duration declaration should exist")
        .type_index;
    let identity = time
        .executables
        .iter()
        .find(|executable| executable.symbol == "std.time.identity")
        .expect("identity executable should exist");
    let passthrough = consumer
        .executables
        .iter()
        .find(|executable| executable.symbol == "std.consumer.passthrough")
        .expect("passthrough executable should exist");

    assert_eq!(
        identity.params[0].ty,
        TypeRefIr::LocalType {
            type_index: duration_type_index,
        }
    );
    assert_eq!(
        identity.return_type,
        TypeRefIr::LocalType {
            type_index: duration_type_index,
        }
    );
    let expected_cross_module = TypeRefIr::PublicationType {
        module_path: "std.time".to_string(),
        type_index: duration_type_index,
    };
    assert_eq!(passthrough.params[0].ty, expected_cross_module);
    assert_eq!(passthrough.return_type, expected_cross_module);
}

#[test]
fn lowers_interface_box_to_local_method_table() {
    let unit = lowered_unit(any_interface_source());
    let make_box = executable(&unit, "make_box");
    let impl_executable_index = unit
        .declarations
        .executables
        .get("HostProvider.name")
        .expect("impl method declaration should exist")
        .executable_index;

    let ExprIr::InterfaceBox {
        interface,
        source: BoxSourceIr::Local {
            concrete_type,
            method_table,
        },
        ..
    } = only_interface_box(make_box)
    else {
        panic!("expected InterfaceBox Local source");
    };

    assert_eq!(&method_table.interface, interface);
    assert_eq!(&method_table.concrete_type, concrete_type);
    assert!(
        matches!(concrete_type, TypeRefIr::LocalType { .. }),
        "box source concrete type should be a local nominal type"
    );
    assert_eq!(method_table.slots.len(), 1);
    let slot = &method_table.slots[0];
    assert_eq!(slot.slot, 0);
    assert_eq!(slot.method_name, "name");
    assert_eq!(
        slot.target.executable_index, impl_executable_index,
        "method table slot must target the local impl method executable"
    );
    assert_eq!(
        slot.target.receiver_call_abi,
        ReceiverCallAbi::ExplicitSelfFirst
    );
    assert_eq!(slot.signature.params.len(), 1);
    assert_eq!(slot.signature.params[0].name, "self");
    assert_eq!(slot.signature.return_type, TypeRefIr::builtin("string"));
    assert!(!slot.method_abi_id.is_empty());
}

#[test]
fn lowers_package_interface_box_to_local_method_table() {
    let unit = lowered_unit_with_package_facts(package_interface_box_source());
    let make_box = executable(&unit, "make_package_box");
    let impl_executable_index = unit
        .declarations
        .executables
        .get("Host.read")
        .expect("impl method declaration should exist")
        .executable_index;

    let ExprIr::InterfaceBox {
        interface,
        source: BoxSourceIr::Local {
            concrete_type,
            method_table,
        },
        ..
    } = only_interface_box(make_box)
    else {
        panic!("expected package InterfaceBox Local source");
    };

    let interface_ty = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .expect("interface ABI id should decode");
    let TypeRefIr::PackageSymbol { symbol } = interface_ty else {
        panic!("package interface box should use PackageSymbol ABI identity");
    };
    assert_eq!(symbol.symbol_path, "Reader");
    assert!(matches!(
        symbol.package,
        PackageRefIr::PackageId { ref package_id } if package_id == PACKAGE_ID
    ));
    assert_eq!(
        interface.canonical_type_args,
        vec![TypeRefIr::builtin("string")]
    );
    assert_eq!(&method_table.interface, interface);
    assert_eq!(&method_table.concrete_type, concrete_type);
    assert_eq!(method_table.slots.len(), 1);
    let slot = &method_table.slots[0];
    assert_eq!(slot.slot, 0);
    assert_eq!(slot.method_name, "read");
    assert_eq!(slot.target.executable_index, impl_executable_index);
    assert_eq!(
        slot.target.receiver_call_abi,
        ReceiverCallAbi::ExplicitSelfFirst
    );
    assert_eq!(slot.signature.params.len(), 2);
    assert_eq!(slot.signature.params[1].name, "fallback");
    assert_eq!(slot.signature.params[1].ty, TypeRefIr::builtin("string"));
    assert_eq!(slot.signature.return_type, TypeRefIr::builtin("string"));
    assert!(!slot.method_abi_id.is_empty());
}

#[test]
fn lowers_any_interface_function_param_to_any_interface_type_ref() {
    let unit = lowered_unit(any_interface_signature_source());
    let accept = executable(&unit, "accept");
    let TypeRefIr::AnyInterface { interface } = &accept.params[0].ty else {
        panic!("any Provider parameter should lower to TypeRefIr::AnyInterface");
    };
    let interface_ty = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .expect("interface ABI id should decode");
    let provider_type_index = unit
        .declarations
        .types
        .get("Provider")
        .expect("Provider declaration should exist")
        .type_index;
    assert_eq!(
        interface_ty,
        TypeRefIr::PublicationType {
            module_path: MODULE.to_string(),
            type_index: provider_type_index,
        }
    );
    assert!(interface.canonical_type_args.is_empty());
}

#[test]
fn exact_package_any_interface_function_param_preserves_package_owner() {
    let unit = lowered_unit_with_package_facts(package_any_interface_signature_source());
    let accept = executable(&unit, "accept_package");
    let TypeRefIr::AnyInterface { interface } = &accept.params[0].ty else {
        panic!("any pkg.Reader<string> parameter should lower to TypeRefIr::AnyInterface");
    };
    let interface_ty = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .expect("interface ABI id should decode");
    let TypeRefIr::PackageSymbol { symbol } = interface_ty else {
        panic!("package interface selector should use PackageSymbol ABI identity");
    };
    assert_eq!(symbol.symbol_path, "Reader");
    assert!(matches!(
        symbol.package,
        PackageRefIr::PackageId { ref package_id } if package_id == PACKAGE_ID
    ));
    assert_eq!(
        interface.canonical_type_args,
        vec![TypeRefIr::builtin("string")]
    );
}

#[test]
fn lowers_any_interface_receiver_call_to_interface_method_target() {
    let unit = lowered_unit(any_interface_source());
    let call_box = executable(&unit, "call_box");
    let boxed = only_interface_box(call_box);
    let ExprIr::InterfaceBox {
        interface,
        source: BoxSourceIr::Local { method_table, .. },
        ..
    } = boxed
    else {
        panic!("expected local InterfaceBox before receiver call");
    };
    let slot = &method_table.slots[0];

    let call = call_box
        .body
        .expressions
        .iter()
        .find_map(|expr| {
            let ExprIr::Call { call } = expr else {
                return None;
            };
            matches!(call.target, CallTargetIr::InterfaceMethod { .. }).then_some(call)
        })
        .expect("provider.name() should lower to InterfaceMethod call");

    let CallTargetIr::InterfaceMethod {
        interface: call_interface,
        method_abi_id,
        slot: call_slot,
    } = &call.target
    else {
        unreachable!("find_map only returns InterfaceMethod calls");
    };
    assert_eq!(call_interface, interface);
    assert_eq!(method_abi_id, &slot.method_abi_id);
    assert_eq!(*call_slot, slot.slot);
    assert_eq!(call.args.len(), 1, "receiver should be the first arg");
    let receiver_arg = &call_box.body.expressions[call.args[0].expression as usize];
    assert!(
        matches!(receiver_arg, ExprIr::LoadSlot { .. }),
        "receiver arg should load the boxed local binding"
    );
}

#[test]
fn exact_receiver_builtin_targets_are_consumed_from_source_facts() {
    let unit = lowered_unit(
        r#"
              function isBefore(left: Date, right: Date) -> bool {
                return left.isBefore(right)
              }

              function epoch(value: Date) -> integer {
                return value.toEpochMilliseconds()
              }

              function millis(value: Duration) -> integer {
                return value.toMilliseconds()
              }

              function now() -> Date {
                return Date.now()
              }

              function sleep() -> void {
                return std.time.sleep(Duration.milliseconds(0))
              }
            "#,
    );
    let targets = unit
        .executables
        .iter()
        .flat_map(|executable| executable.body.expressions.iter())
        .filter_map(|expression| match expression {
            ExprIr::Call {
                call:
                    CallIr {
                        target: CallTargetIr::ReceiverBuiltin { op },
                        ..
                    },
            } => Some(op.canonical_key),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        targets,
        BTreeSet::from([
            "receiver:Date.isBefore@1",
            "receiver:Date.toEpochMilliseconds@1",
            "receiver:Duration.toMilliseconds@1",
        ])
    );
    for name in ["isBefore", "epoch", "millis", "now"] {
        assert!(
            !executable(&unit, name).may_suspend,
            "{name} should consume exact non-suspending callable semantics"
        );
    }
    assert!(
        executable(&unit, "sleep").may_suspend,
        "sleep should consume its exact may-suspend descriptor"
    );
}

#[test]
fn typed_contract_call_site_lowers_to_canonical_service_call_without_legacy_operation_abi() {
    let source = r#"
          function run() -> void {
            echo/ping()
          }
        "#;
    let operation_id = ContractOperationId::new("operation:ping");
    let protocol = ServiceProtocolIdentity::new("protocol:echo");
    let contract_requirement = ContractRequirement {
        alias: "echo".to_string(),
        service_id: "example.echo".to_string(),
        contract_version: "1.0.0".to_string(),
        expected_protocol_identity: protocol.clone(),
    };
    let expression = skiff_compiler_source::ExpressionKey::new(
        MODULE,
        skiff_compiler_source::ExpressionOwnerKey::Function("run".to_string()),
        0,
    );
    let targets = skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
        expression,
        skiff_compiler_source::ResolvedCallTarget::ContractOperation {
            contract_requirement,
            contract_operation_id: operation_id.clone(),
        },
    )]));
    let service_calls = crate::lower_service_calls(&targets).unwrap();
    let service_aliases = BTreeSet::from(["echo".to_string()]);
    let ast = parse_source(source).unwrap();
    let unit = compile_parsed_source_file_ir_unit_with_lowering_context(
        ast,
        source,
        "internal/any_lowering.skiff",
        MODULE,
        "package",
        &SourceFileLoweringContext {
            service_dependency_aliases: &service_aliases,
            service_calls: Some(&service_calls),
            ..SourceFileLoweringContext::none()
        },
    )
    .unwrap();

    validate_file_ir_service_calls(&unit).unwrap();
    assert_eq!(unit.external_refs.service_call_refs.len(), 1);
    assert_eq!(
        unit.external_refs.service_call_refs[0].contract_operation_id,
        operation_id
    );
    assert_eq!(
        unit.external_refs.service_call_refs[0].expected_protocol_identity,
        protocol
    );
    let run = executable(&unit, "run");
    assert!(run.body.expressions.iter().any(|expression| matches!(
        expression,
        ExprIr::Call { call }
            if matches!(call.target, CallTargetIr::ServiceCall { .. })
    )));
    assert!(!run.body.expressions.iter().any(|expression| matches!(
        expression,
        ExprIr::Call { call }
            if matches!(call.target, CallTargetIr::ServiceDependencySymbol { .. })
    )));
    let wire = serde_json::to_string(&unit).unwrap();
    assert!(!wire.contains("operationAbiId"));
    assert!(!wire.contains("serviceDependencySymbols"));
}

fn package_call_source() -> &'static str {
    r#"
          function run() -> void {
            utils/format()
          }
        "#
}

fn package_call_expression() -> skiff_compiler_source::ExpressionKey {
    skiff_compiler_source::ExpressionKey::new(
        MODULE,
        skiff_compiler_source::ExpressionOwnerKey::Function("run".to_string()),
        0,
    )
}

fn lower_package_call(
    package_aliases: &BTreeMap<String, Vec<String>>,
    targets: &skiff_compiler_source::ResolvedCallTargetFacts,
) -> skiff_syntax::error::Result<FileIrUnit> {
    initialize_test_prelude();
    let source = package_call_source();
    let ast = parse_source(source)?;
    compile_parsed_source_file_ir_unit_with_lowering_context(
        ast,
        source,
        "internal/any_lowering.skiff",
        MODULE,
        "package",
        &SourceFileLoweringContext {
            package_aliases,
            resolved_call_targets: targets,
            ..SourceFileLoweringContext::none()
        },
    )
}

fn lower_local_function_call(
    targets: &skiff_compiler_source::ResolvedCallTargetFacts,
) -> skiff_syntax::error::Result<FileIrUnit> {
    initialize_test_prelude();
    let source = r#"
          function helper() -> string {
            return "ok"
          }

          function run() -> string {
            return helper()
          }
        "#;
    compile_parsed_source_file_ir_unit_with_lowering_context(
        parse_source(source)?,
        source,
        "internal/any_lowering.skiff",
        MODULE,
        "package",
        &SourceFileLoweringContext {
            resolved_call_targets: targets,
            ..SourceFileLoweringContext::none()
        },
    )
}

fn local_run_call_expression() -> skiff_compiler_source::ExpressionKey {
    skiff_compiler_source::ExpressionKey::new(
        MODULE,
        skiff_compiler_source::ExpressionOwnerKey::Function("run".to_string()),
        0,
    )
}

#[test]
fn unresolved_local_function_does_not_fall_back_to_name_lookup() {
    let targets = skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
        local_run_call_expression(),
        skiff_compiler_source::ResolvedCallTarget::Unknown {
            reason: skiff_compiler_source::UnknownCallTargetReason::UnresolvedName,
        },
    )]));
    let error = lower_local_function_call(&targets)
        .expect_err("an unresolved local target must fail before File IR")
        .to_string();
    assert!(
        error.contains("callee `helper` is not resolved"),
        "unexpected unresolved target diagnostic: {error}"
    );
}

#[test]
fn typed_local_function_index_mismatch_fails_before_file_ir() {
    let targets = skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
        local_run_call_expression(),
        skiff_compiler_source::ResolvedCallTarget::LocalFunction {
            source_callable: skiff_compiler_source::SourceSymbolKey::new(MODULE, "helper"),
            executable_index: 99,
        },
    )]));
    let error = lower_local_function_call(&targets)
        .expect_err("a mutated executable index must fail before File IR")
        .to_string();
    assert!(
        error.contains("canonical local index is 0"),
        "unexpected executable index diagnostic: {error}"
    );
}

#[test]
fn typed_package_call_site_without_exact_signature_fails_closed() {
    let expression = package_call_expression();
    let expected_local_abi = PackageLocalAbiIdentity::new("local-abi:must-not-enter-call-site");
    let package_callable_id = PackageCallableId::new("callable:utils.format");
    let targets = skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
        expression,
        skiff_compiler_source::ResolvedCallTarget::DependencyPackageFunction {
            package_requirement_alias: "utils".to_string(),
            compiler_owned: false,
            package_callable_id: package_callable_id.clone(),
            expected_local_abi: expected_local_abi.clone(),
            exact_signature: None,
            inout_parameters: BTreeMap::new(),
        },
    )]));
    let package_aliases = BTreeMap::from([("utils".to_string(), vec![String::new()])]);
    let error = lower_package_call(&package_aliases, &targets)
        .expect_err("missing exact package signature must fail closed");
    assert!(error
        .to_string()
        .contains("package-direct target `callable:utils.format` has no exact signature"));
}

#[test]
fn dependency_exact_signature_controls_lowered_suspend_flag_without_synthetic_calls() {
    let lower = |may_suspend| {
        let targets =
            skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
                package_call_expression(),
                skiff_compiler_source::ResolvedCallTarget::DependencyPackageFunction {
                    package_requirement_alias: "utils".to_string(),
                    compiler_owned: false,
                    package_callable_id: PackageCallableId::new("callable:utils.format"),
                    expected_local_abi: PackageLocalAbiIdentity::new("local-abi:utils"),
                    exact_signature: Some(skiff_artifact_model::PackageCallableSignature {
                        type_params: Vec::new(),
                        parameters: Vec::new(),
                        return_type: skiff_artifact_model::PackageTypeRef::Local {
                            local_type: TypeRefIr::builtin("void"),
                        },
                        may_suspend,
                    }),
                    inout_parameters: BTreeMap::new(),
                },
            )]));
        lower_package_call(
            &BTreeMap::from([("utils".to_string(), vec![String::new()])]),
            &targets,
        )
        .unwrap()
    };
    let non_suspending = lower(false);
    let suspending = lower(true);
    let non_suspending_run = executable(&non_suspending, "run");
    let suspending_run = executable(&suspending, "run");

    assert!(!non_suspending_run.may_suspend);
    assert!(suspending_run.may_suspend);
    assert_eq!(
        non_suspending_run.body, suspending_run.body,
        "conservative suspension changes only the executable summary, not the call body"
    );
    assert_eq!(
        suspending_run
            .body
            .expressions
            .iter()
            .filter(|expression| matches!(expression, ExprIr::Call { .. }))
            .count(),
        1,
        "suspension inference must not inject a synthetic runtime call"
    );
}

#[test]
fn known_package_call_without_typed_target_fails_closed() {
    let package_aliases = BTreeMap::from([("utils".to_string(), vec![String::new()])]);
    let error = lower_package_call(
        &package_aliases,
        &skiff_compiler_source::ResolvedCallTargetFacts::empty(),
    )
    .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("package dependency call `utils/format`"));
    assert!(message.contains("missing ResolvedCallTargetFacts entry"));
}

#[test]
fn known_package_call_with_unknown_typed_target_fails_closed() {
    let targets = skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
        package_call_expression(),
        skiff_compiler_source::ResolvedCallTarget::Unknown {
            reason: skiff_compiler_source::UnknownCallTargetReason::UnresolvedName,
        },
    )]));
    let package_aliases = BTreeMap::from([("utils".to_string(), vec![String::new()])]);
    let error = lower_package_call(&package_aliases, &targets).unwrap_err();

    let message = error.to_string();
    assert!(message.contains("package dependency call `utils/format`"));
    assert!(message.contains("Unknown(UnresolvedName)"));
}

#[test]
fn package_call_target_alias_must_match_callee_root() {
    let targets = skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
        package_call_expression(),
        skiff_compiler_source::ResolvedCallTarget::DependencyPackageFunction {
            package_requirement_alias: "other".to_string(),
            compiler_owned: false,
            package_callable_id: PackageCallableId::new("callable:other.format"),
            expected_local_abi: PackageLocalAbiIdentity::new("local-abi:other"),
            exact_signature: None,
            inout_parameters: BTreeMap::new(),
        },
    )]));
    let package_aliases = BTreeMap::from([
        ("other".to_string(), vec![String::new()]),
        ("utils".to_string(), vec![String::new()]),
    ]);
    let error = lower_package_call(&package_aliases, &targets).unwrap_err();

    let message = error.to_string();
    assert!(message.contains("typed package target names dependency `other`"));
    assert!(message.contains("callee root is `utils`"));
}

#[test]
fn record_pattern_match_preserves_kind_literal_and_field_bindings() {
    let unit = lowered_unit(
        r#"
            function run(status: { kind: string, detail: string }) -> string {
              match status {
                { kind: "succeeded", detail } => {
                  return detail
                }
                { kind: "failed" } => {
                  return "failed"
                }
                _ => {
                  return "other"
                }
              }
            }
        "#,
    );
    let run = executable(&unit, "run");
    let match_statement = run
        .body
        .statements
        .iter()
        .find(|statement| matches!(statement, StmtIr::Match { .. }))
        .expect("expected match statement");
    let StmtIr::Match { arms, .. } = match_statement else {
        unreachable!();
    };
    assert_eq!(arms.len(), 3, "discriminated union arms must stay ordered");

    let PatternIr::Record { fields } = &arms[0].pattern else {
        panic!("`{{ kind: \"succeeded\", detail }}` must lower to a record pattern");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "kind");
    assert_eq!(
        fields[0].pattern,
        PatternIr::Literal {
            value: LiteralIr::String {
                value: "succeeded".to_string(),
            },
        },
        "record pattern kind discriminant must preserve its literal"
    );
    let PatternIr::Binding { slot } = fields[1].pattern else {
        panic!("bare record pattern field must lower to a binding");
    };
    assert!(
        run.slots.slots.iter().any(|declared| {
            declared.index == slot
                && declared.name == "detail"
                && declared.kind == SlotKind::Pattern
        }),
        "record pattern bare field must declare its slot in the executable layout"
    );

    let PatternIr::Record { fields } = &arms[1].pattern else {
        panic!("`{{ kind: \"failed\" }}` must lower to a record pattern");
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "kind");
    assert!(matches!(
        &fields[0].pattern,
        PatternIr::Literal {
            value: LiteralIr::String { value },
        } if value == "failed"
    ));

    assert!(matches!(arms[2].pattern, PatternIr::Wildcard));
}

#[test]
fn record_pattern_nested_field_patterns_lower_recursively() {
    let unit = lowered_unit(
        r#"
            function run(payload: { kind: string, body: { state: string } }) -> string {
              match payload {
                { kind: "ok", body: { state } } => {
                  return state
                }
                _ => {
                  return "other"
                }
              }
            }
        "#,
    );
    let run = executable(&unit, "run");
    let match_statement = run
        .body
        .statements
        .iter()
        .find(|statement| matches!(statement, StmtIr::Match { .. }))
        .expect("expected match statement");
    let StmtIr::Match { arms, .. } = match_statement else {
        unreachable!();
    };
    let PatternIr::Record { fields } = &arms[0].pattern else {
        panic!("outer pattern must lower to a record pattern");
    };
    let PatternIr::Record {
        fields: body_fields,
    } = &fields[1].pattern
    else {
        panic!("nested `{{ state }}` must lower to a record pattern");
    };
    assert_eq!(body_fields.len(), 1);
    assert!(matches!(&body_fields[0].pattern, PatternIr::Binding { .. }));
    assert!(
        run.slots.slots.iter().any(|slot| slot.name == "state"),
        "nested record pattern bare field must declare its slot"
    );
}

mod interface_execution;
mod object_materialization;

fn provider_contract_artifact() -> (
    skiff_artifact_model::PackageArtifact,
    skiff_artifact_model::FileIrUnit,
) {
    let package_id = "example.com/engine";
    let mut file = skiff_artifact_model::FileIrUnit::empty("model", format!("{package_id}:source"));
    file.type_table.push(TypeDeclIr {
        name: "AgentThread".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([
                ("id".to_string(), TypeRefIr::builtin("string")),
                ("status".to_string(), TypeRefIr::builtin("string")),
                ("updatedAt".to_string(), TypeRefIr::builtin("string")),
            ]),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        "AgentThread".to_string(),
        skiff_artifact_model::TypeDeclarationIr {
            type_index: 0,
            symbol: "model.AgentThread".to_string(),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        "AgentThread".to_string(),
        skiff_artifact_model::DbDeclarationIr {
            type_ref: TypeRefIr::LocalType { type_index: 0 },
            type_name: "model.AgentThread".to_string(),
            collection_name: None,
            implements: None,
            identity_fields: BTreeMap::from([
                ("id".to_string(), TypeRefIr::builtin("string")),
                ("status".to_string(), TypeRefIr::builtin("string")),
                ("updatedAt".to_string(), TypeRefIr::builtin("string")),
            ]),
            kind: skiff_artifact_model::DbObjectKindIr::Contract,
            key: skiff_artifact_model::DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: vec![
                skiff_artifact_model::DbObjectFieldIr {
                    name: "status".to_string(),
                    ty: TypeRefIr::builtin("string"),
                    storage: skiff_artifact_model::DbFieldStorageIr::Identity,
                },
                skiff_artifact_model::DbObjectFieldIr {
                    name: "updatedAt".to_string(),
                    ty: TypeRefIr::builtin("string"),
                    storage: skiff_artifact_model::DbFieldStorageIr::Identity,
                },
            ],
            retention: None,
            leases: Vec::new(),
            indexes: vec![skiff_artifact_model::DbIndexIr {
                name: "byStatusUpdated".to_string(),
                unique: false,
                fields: vec![
                    skiff_artifact_model::DbIndexFieldIr {
                        field: skiff_artifact_model::FieldPathIr {
                            text: "status".to_string(),
                            segments: vec!["status".to_string()],
                        },
                        direction: skiff_artifact_model::DbIndexDirectionIr::Asc,
                    },
                    skiff_artifact_model::DbIndexFieldIr {
                        field: skiff_artifact_model::FieldPathIr {
                            text: "updatedAt".to_string(),
                            segments: vec!["updatedAt".to_string()],
                        },
                        direction: skiff_artifact_model::DbIndexDirectionIr::Desc,
                    },
                ],
            }],
            source_span: None,
        },
    );
    skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
    let file_ref = provider_file_ref(&file);
    let descriptor = file.type_table[0].descriptor.clone();
    let mut artifact = skiff_artifact_model::PackageArtifact {
        schema_version: skiff_artifact_model::PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: skiff_artifact_model::PackageBuildId::new("unassigned"),
        files: vec![file_ref.clone()],
        static_resources: Vec::new(),
        package_local_abi: skiff_artifact_model::PackageLocalAbi {
            local_abi_identity: skiff_artifact_model::PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::from([(
                "model.AgentThread".to_string(),
                skiff_artifact_model::PackageLocalAbiSymbol::Type {
                    local_type_id: format!("type:{package_id}:top-level:model.AgentThread"),
                    descriptor: descriptor.clone(),
                    is_alias: false,
                    is_interface: false,
                    type_params: Vec::new(),
                    interface_methods: Vec::new(),
                    actor: None,
                },
            )]),
        },
        package_schema_index: skiff_artifact_model::PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_model::PackageSchemaIndexIdentity::new(
                "unassigned",
            ),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: skiff_artifact_model::PackageImplementationLinks {
            types: BTreeMap::from([(
                "model.AgentThread".to_string(),
                skiff_artifact_model::TypeExport {
                    file: file_ref,
                    type_index: 0,
                    symbol: "model.AgentThread".to_string(),
                    is_interface: false,
                    descriptor: Some(descriptor),
                    type_params: Vec::new(),
                    interface_methods: Vec::new(),
                    actor: None,
                },
            )]),
            ..skiff_artifact_model::PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: skiff_artifact_model::PackageRuntimeRequirements {
            config: Vec::new(),
        },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
        bytecode: None,
    };
    artifact.package_schema_index.package_schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(package_id, &BTreeMap::new())
            .unwrap();
    skiff_artifact_identity::assign_package_artifact_identities(&mut artifact).unwrap();
    (artifact, file)
}

fn provider_object_artifact() -> (
    skiff_artifact_model::PackageArtifact,
    skiff_artifact_model::FileIrUnit,
) {
    let package_id = "example.com/provider";
    let mut file = skiff_artifact_model::FileIrUnit::empty("model", format!("{package_id}:source"));
    file.type_table.push(TypeDeclIr {
        name: "Session".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([
                ("id".to_string(), TypeRefIr::builtin("string")),
                ("value".to_string(), TypeRefIr::builtin("string")),
            ]),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        "Session".to_string(),
        skiff_artifact_model::TypeDeclarationIr {
            type_index: 0,
            symbol: "model.Session".to_string(),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        "Session".to_string(),
        skiff_artifact_model::DbDeclarationIr {
            type_ref: TypeRefIr::LocalType { type_index: 0 },
            type_name: "model.Session".to_string(),
            collection_name: Some("sessions".to_string()),
            implements: None,
            identity_fields: std::collections::BTreeMap::new(),
            kind: skiff_artifact_model::DbObjectKindIr::Object,
            key: skiff_artifact_model::DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: vec![skiff_artifact_model::DbObjectFieldIr {
                name: "value".to_string(),
                ty: TypeRefIr::builtin("string"),
                storage: skiff_artifact_model::DbFieldStorageIr::Identity,
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
    let file_ref = provider_file_ref(&file);
    let descriptor = file.type_table[0].descriptor.clone();
    let mut artifact = skiff_artifact_model::PackageArtifact {
        schema_version: skiff_artifact_model::PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: skiff_artifact_model::PackageBuildId::new("unassigned"),
        files: vec![file_ref.clone()],
        static_resources: Vec::new(),
        package_local_abi: skiff_artifact_model::PackageLocalAbi {
            local_abi_identity: skiff_artifact_model::PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::from([(
                "model.Session".to_string(),
                skiff_artifact_model::PackageLocalAbiSymbol::Type {
                    local_type_id: format!("type:{package_id}:top-level:model.Session"),
                    descriptor: descriptor.clone(),
                    is_alias: false,
                    is_interface: false,
                    type_params: Vec::new(),
                    interface_methods: Vec::new(),
                    actor: None,
                },
            )]),
        },
        package_schema_index: skiff_artifact_model::PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_model::PackageSchemaIndexIdentity::new(
                "unassigned",
            ),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: skiff_artifact_model::PackageImplementationLinks {
            types: BTreeMap::from([(
                "model.Session".to_string(),
                skiff_artifact_model::TypeExport {
                    file: file_ref,
                    type_index: 0,
                    symbol: "model.Session".to_string(),
                    is_interface: false,
                    descriptor: Some(descriptor),
                    type_params: Vec::new(),
                    interface_methods: Vec::new(),
                    actor: None,
                },
            )]),
            ..skiff_artifact_model::PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: skiff_artifact_model::PackageRuntimeRequirements {
            config: Vec::new(),
        },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
        bytecode: None,
    };
    artifact.package_schema_index.package_schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(package_id, &BTreeMap::new())
            .unwrap();
    skiff_artifact_identity::assign_package_artifact_identities(&mut artifact).unwrap();
    (artifact, file)
}

fn provider_file_ref(file: &skiff_artifact_model::FileIrUnit) -> skiff_artifact_model::FileIrRef {
    skiff_artifact_model::FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    }
}

fn lowered_units_with_provider_contract(
    source_text: &str,
    provider: &skiff_artifact_model::PackageArtifact,
    provider_file: &skiff_artifact_model::FileIrUnit,
    contracts_only: bool,
    top_level_alias: &str,
) -> std::result::Result<Vec<FileIrUnit>, String> {
    initialize_test_prelude();
    let root = PathBuf::from("/test");
    let source = CompilerSourceFile::parse(
        PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        source_text.to_string(),
        "main.skiff",
    )
    .map_err(|error| error.to_string())?;
    let parsed_sources =
        parse_publication_sources(&root, &[source]).map_err(|error| error.to_string())?;
    let package_aliases = BTreeMap::from([("provider".to_string(), vec![String::new()])]);
    let mut dependency = PackageDependency::id("example.com/engine");
    dependency.alias = Some("provider".to_string());
    let package_dependencies = vec![dependency];
    let foreign = skiff_compiler_source::foreign_package_db_metadata_index(&[
        skiff_compiler_source::ForeignPackageDbDependency {
            primary_alias: "provider",
            top_level_alias,
            contracts_only,
            artifact: provider,
            files: std::slice::from_ref(provider_file),
        },
    ])
    .map_err(|error| error.to_string())?;
    let dependency_analysis = skiff_compiler_source::SourceDependencyAnalysisInput::default()
        .with_foreign_db_metadata(foreign);
    let model = build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources: Vec::new(),
            diagnostic_root: &root,
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            package_artifacts: Some(std::slice::from_ref(provider)),
            policy: PackageCompilePolicy::new("example.com/host"),
        },
        &dependency_analysis,
    )
    .map_err(|error| error.to_string())?;
    crate::lower(&model)
        .map_err(|error| error.to_string())
        .map(|lowered| lowered.file_ir_units().to_vec())
}

#[test]
fn db_object_implements_cross_package_contract_lowers_with_coverage() {
    let (provider, provider_file) = provider_contract_artifact();
    let units = lowered_units_with_provider_contract(
        r#"
            type Thread {
              id: string,
              status: string,
              updatedAt: string
            }
            db object Thread implements provider/model.AgentThread {
              primary key(id)
              index byStatusUpdated(status, updatedAt desc)
            }
        "#,
        &provider,
        &provider_file,
        true,
        "provider",
    )
    .expect("covered db object implements contract should lower");

    let declaration = units[0]
        .declarations
        .db
        .get("Thread")
        .expect("validated db object should lower");
    assert_eq!(
        declaration.kind,
        skiff_artifact_model::DbObjectKindIr::Object
    );
    assert_eq!(declaration.collection_name.as_deref(), Some("Thread"));
    assert_eq!(
        declaration.implements,
        Some(TypeRefIr::PackageSymbol {
            symbol: skiff_artifact_model::PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "provider".to_string(),
                },
                symbol_path: "model.AgentThread".to_string(),
                abi_expectation: None,
            },
        })
    );
    assert_eq!(declaration.key.name, "id");
    assert!(declaration
        .fields
        .iter()
        .any(|field| field.name == "updatedAt"));
    assert_eq!(declaration.indexes.len(), 1);
    assert_eq!(declaration.indexes[0].name, "byStatusUpdated");
}

#[test]
fn db_object_implements_accepts_dot_spelled_contract_ref() {
    let (provider, provider_file) = provider_contract_artifact();
    let units = lowered_units_with_provider_contract(
        r#"
            type Thread {
              id: string,
              status: string,
              updatedAt: string
            }
            db object Thread implements provider.model.AgentThread {
              primary key(id)
            }
        "#,
        &provider,
        &provider_file,
        true,
        "provider",
    )
    .expect("dot spelled contract ref should resolve the same contract");

    let declaration = units[0]
        .declarations
        .db
        .get("Thread")
        .expect("validated db object should lower");
    assert_eq!(
        declaration.implements,
        Some(TypeRefIr::PackageSymbol {
            symbol: skiff_artifact_model::PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "provider".to_string(),
                },
                symbol_path: "model.AgentThread".to_string(),
                abi_expectation: None,
            },
        })
    );
}

#[test]
fn db_object_implements_missing_contract_field_fails_closed() {
    let (provider, provider_file) = provider_contract_artifact();
    let error = lowered_units_with_provider_contract(
        r#"
            type Thread {
              id: string,
              status: string
            }
            db object Thread implements provider/model.AgentThread {
              primary key(id)
            }
        "#,
        &provider,
        &provider_file,
        true,
        "provider",
    )
    .expect_err("missing contract field must fail closed");
    assert!(
        error.contains("missing contract fields"),
        "unexpected error: {error}"
    );
    assert!(error.contains("updatedAt"), "unexpected error: {error}");
}

#[test]
fn db_object_implements_field_type_mismatch_fails_closed() {
    let (provider, provider_file) = provider_contract_artifact();
    let error = lowered_units_with_provider_contract(
        r#"
            type Thread {
              id: string,
              status: number,
              updatedAt: string
            }
            db object Thread implements provider/model.AgentThread {
              primary key(id)
            }
        "#,
        &provider,
        &provider_file,
        true,
        "provider",
    )
    .expect_err("contract field type mismatch must fail closed");
    assert!(
        error.contains("different schema identity"),
        "unexpected error: {error}"
    );
    assert!(error.contains("status"), "unexpected error: {error}");
}

#[test]
fn db_object_implements_key_mismatch_fails_closed() {
    let (provider, provider_file) = provider_contract_artifact();
    let error = lowered_units_with_provider_contract(
        r#"
            type Thread {
              id: number,
              status: string,
              updatedAt: string
            }
            db object Thread implements provider/model.AgentThread {
              primary key(id)
            }
        "#,
        &provider,
        &provider_file,
        true,
        "provider",
    )
    .expect_err("contract key mismatch must fail closed");
    assert!(
        error.contains("different schema identity"),
        "unexpected error: {error}"
    );
    assert!(error.contains("id"), "unexpected error: {error}");
}

#[test]
fn db_object_implements_storage_mapping_mismatch_fails_closed() {
    let (provider, provider_file) = provider_contract_artifact();
    let error = lowered_units_with_provider_contract(
        r#"
            type Thread {
              id: string,
              status: string,
              updatedAt: string
            }
            db object Thread implements provider/model.AgentThread {
              primary key(id)
              storage status using encrypted
            }
        "#,
        &provider,
        &provider_file,
        true,
        "provider",
    )
    .expect_err("contract storage mismatch must fail closed");
    assert!(
        error.contains("different storage mapping"),
        "unexpected error: {error}"
    );
    assert!(error.contains("status"), "unexpected error: {error}");
}

#[test]
fn db_object_implements_non_contract_db_object_fails_closed() {
    let (provider, provider_file) = provider_object_artifact();
    let error = lowered_units_with_provider_contract(
        r#"
            type Session {
              id: string,
              value: string
            }
            db object Session implements providerImpl/model.Session {
              primary key(id)
            }
        "#,
        &provider,
        &provider_file,
        false,
        "providerImpl",
    )
    .expect_err("implements target that is a plain db object must fail closed");
    assert!(
        error.contains("not a db contract"),
        "unexpected error: {error}"
    );
}

#[test]
fn db_object_implements_plain_type_or_interface_fails_closed() {
    let (provider, provider_file) = provider_contract_artifact();
    let error = lowered_units_with_provider_contract(
        r#"
            type Thread {
              id: string,
              status: string,
              updatedAt: string
            }
            db object Thread implements provider/model.Reader {
              primary key(id)
            }
        "#,
        &provider,
        &provider_file,
        true,
        "provider",
    )
    .expect_err("implements target without a db contract attachment must fail closed");
    assert!(
        error.contains("does not resolve to a db contract declaration"),
        "unexpected error: {error}"
    );
}

#[test]
fn db_object_implements_local_contract_fails_closed() {
    let (provider, provider_file) = provider_contract_artifact();
    let error = lowered_units_with_provider_contract(
        r#"
            type AgentThread {
              id: string,
              status: string
            }
            db contract AgentThread {
              primary key(id)
            }
            type Thread {
              id: string,
              status: string
            }
            db object Thread implements AgentThread {
              primary key(id)
            }
        "#,
        &provider,
        &provider_file,
        true,
        "provider",
    )
    .expect_err("same-package contract reference must fail closed");
    assert!(
        error.contains("must be a cross-package contract reference"),
        "unexpected error: {error}"
    );
}

#[test]
fn db_object_implements_unknown_contract_target_fails_closed() {
    let (provider, provider_file) = provider_contract_artifact();
    let error = lowered_units_with_provider_contract(
        r#"
            type Thread {
              id: string,
              status: string,
              updatedAt: string
            }
            db object Thread implements provider/model.MissingThread {
              primary key(id)
            }
        "#,
        &provider,
        &provider_file,
        true,
        "provider",
    )
    .expect_err("unknown contract target must fail closed");
    assert!(
        error.contains("does not resolve to a db contract declaration"),
        "unexpected error: {error}"
    );
}
