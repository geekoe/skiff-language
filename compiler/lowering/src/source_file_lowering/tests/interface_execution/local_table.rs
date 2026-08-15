use std::{collections::BTreeMap, path::PathBuf};

use crate::{
    file_ir::{BoxSourceIr, CallTargetIr, ExecutableIr, ExprIr},
    source_unit_lowering::symbol,
};
use skiff_artifact_model::{FileIrUnit, ReceiverCallAbi};
use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_source::{
    build_package_from_parsed_sources, parsed_sources::parse_publication_sources,
    prelude_registry::initialize_prelude_registry, source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageCompilePolicy, PackageDependency,
};

const MODULE: &str = "internal.interface_local_table";

fn lowered_unit(source_text: &str) -> FileIrUnit {
    lowered_unit_result(source_text).expect("publication should lower")
}

fn lowered_unit_result(source_text: &str) -> Result<FileIrUnit, String> {
    initialize_test_prelude();
    let root = PathBuf::from("/test");
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/interface_local_table.skiff"),
        MODULE.to_string(),
        false,
        false,
        source_text.to_string(),
        "internal/interface_local_table.skiff",
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
        policy: PackageCompilePolicy::new("example.com/interface-local-table"),
    })
    .map_err(|error| error.to_string())?;
    let lowered = crate::lower(&model).map_err(|error| error.to_string())?;
    lowered
        .file_ir_units()
        .first()
        .cloned()
        .ok_or_else(|| "one File IR unit should be emitted".to_string())
}

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

fn executable<'a>(unit: &'a FileIrUnit, name: &str) -> &'a ExecutableIr {
    let expected_symbol = symbol(MODULE, name);
    unit.executables
        .iter()
        .find(|executable| executable.symbol == expected_symbol)
        .unwrap_or_else(|| panic!("missing executable `{expected_symbol}`"))
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
fn local_interface_method_slots_are_dense_from_zero_and_call_slot_is_aligned() {
    let unit = lowered_unit(
        r#"
          interface Reader {
            function first(self: Self) -> string
            function second(self: Self) -> string
          }

          type Impl implements Reader {
            value: string,
          }

          impl Impl {
            function first() -> string {
              return self.value
            }
            function second() -> string {
              return self.value
            }
          }

          function call_second() -> string {
            final reader = Impl { value: "interface-ok" } as Reader
            return reader.second()
          }
        "#,
    );
    let call_box = executable(&unit, "call_second");
    let ExprIr::InterfaceBox {
        interface,
        source: BoxSourceIr::Local {
            concrete_type,
            method_table,
        },
        ..
    } = only_interface_box(call_box)
    else {
        panic!("expected InterfaceBox Local source");
    };

    assert_eq!(&method_table.interface, interface);
    assert_eq!(&method_table.concrete_type, concrete_type);
    assert_eq!(method_table.slots.len(), 2);
    assert_eq!(method_table.slots[0].slot, 0);
    assert_eq!(method_table.slots[0].method_name, "first");
    assert_eq!(method_table.slots[1].slot, 1);
    assert_eq!(method_table.slots[1].method_name, "second");
    assert_eq!(
        method_table.slots[1].target.receiver_call_abi,
        ReceiverCallAbi::ExplicitSelfFirst
    );

    let call = call_box
        .body
        .expressions
        .iter()
        .find_map(|expression| {
            let ExprIr::Call { call } = expression else {
                return None;
            };
            matches!(call.target, CallTargetIr::InterfaceMethod { .. }).then_some(call)
        })
        .expect("reader.second() should lower to InterfaceMethod call");
    let CallTargetIr::InterfaceMethod {
        interface: call_interface,
        method_abi_id,
        slot: call_slot,
    } = &call.target
    else {
        unreachable!("find_map only returns InterfaceMethod calls");
    };
    assert_eq!(call_interface, interface);
    assert_eq!(method_abi_id, &method_table.slots[1].method_abi_id);
    assert_eq!(*call_slot, 1);
}

#[test]
fn local_interface_impl_throw_keeps_exact_throw_expression() {
    let unit = lowered_unit(
        r#"
          interface Reader {
            function label(self: Self) -> string
          }

          type Failure {
            message: string,
          }

          type Impl implements Reader {
            value: string,
          }

          impl Impl {
            function label() -> string {
              return throw Failure { message: "boom" }
            }
          }

          function make_box() -> void {
            final reader = Impl { value: "interface-throw" } as Reader
          }
        "#,
    );
    let impl_method = executable(&unit, "Impl.label");
    assert!(
        impl_method
            .body
            .expressions
            .iter()
            .any(|expression| matches!(expression, ExprIr::Throw { .. })),
        "local interface implementation must preserve its ordinary throw expression"
    );

    let make_box = executable(&unit, "make_box");
    let ExprIr::InterfaceBox {
        source: BoxSourceIr::Local { method_table, .. },
        ..
    } = only_interface_box(make_box)
    else {
        panic!("expected InterfaceBox Local source");
    };
    let declaration = unit
        .declarations
        .executables
        .get("Impl.label")
        .expect("local interface implementation declaration should exist");
    assert_eq!(
        method_table.slots[0].target.executable_index, declaration.executable_index,
        "throwing local method slot must target the exact impl executable"
    );
}

#[test]
fn local_interface_call_preserves_source_pending_effect() {
    let unit = lowered_unit(
        r#"
          interface Pinger {
            function ping(self: Self) -> void
          }

          type Impl implements Pinger {
            value: string,
          }

          impl Impl {
            function ping() -> void {
              return std.time.sleep(Duration.milliseconds(0))
            }
          }

          function call_ping() -> void {
            final pinger = Impl { value: "interface-pending" } as Pinger
            pinger.ping()
          }
        "#,
    );

    assert!(
        executable(&unit, "Impl.ping").may_suspend,
        "suspending local interface implementation must retain may_suspend"
    );
    assert!(
        executable(&unit, "call_ping").may_suspend,
        "caller through a local interface method must retain the Pending source effect"
    );
}

#[test]
fn local_interface_wrong_carrier_and_bad_signature_fail_closed() {
    let wrong_carrier = lowered_unit_result(
        r#"
          interface Reader {
            function read(self: Self) -> string
          }

          type Other {
            value: string,
          }

          impl Other {
            function read() -> string { return self.value }
          }

          function run() -> void {
            final reader = Other { value: "x" } as Reader
          }
        "#,
    )
    .expect_err("wrong local carrier must fail before File IR")
    .to_string();
    assert!(
        wrong_carrier.contains("does not explicitly implement"),
        "unexpected wrong carrier diagnostic: {wrong_carrier}"
    );

    let bad_signature = lowered_unit_result(
        r#"
          interface Reader {
            function read(self: Self) -> string
          }

          type Impl implements Reader {
            value: string,
          }

          impl Impl {
            function read() -> number { return 1 }
          }

          function run() -> void {
            final reader = Impl { value: "x" } as Reader
          }
        "#,
    )
    .expect_err("bad local interface signature must fail before File IR")
    .to_string();
    assert!(
        bad_signature.contains("signature does not match"),
        "unexpected bad signature diagnostic: {bad_signature}"
    );
}
