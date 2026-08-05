use std::{collections::BTreeMap, sync::Arc};

use skiff_runtime_linked_program::{
    linked::{DbDeclarationIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr},
    types::anonymous_type_decl,
    ExecutableAddr, FileAddr, FileDeclarations, FileLinkTargets, LinkedFileUnit,
    LinkedInterfaceInstantiationRef, LinkedTypeDescriptor, LinkedTypeRef, RuntimeExecutionPackage,
    RuntimeTypeContext, ServiceSymbolKey, ServiceSymbolRef, TypeAddr, UnitAddr,
};
use skiff_runtime_linked_type_plan::{PlanContext, ProgramTypeView};

use super::*;

#[test]
fn package_db_object_symbol_declaration_in_another_file_generates_runtime_plans() {
    let mut types = RuntimeTypeContext::default();
    register_exported_record_with_any_interface(
        &mut types,
        "tools",
        "AgentRuntimeBindings",
        0,
        BTreeMap::from([(
            "events".to_string(),
            any_interface("events.AgentEventReceiver"),
        )]),
    );

    let service_files: Vec<Arc<LinkedFileUnit>> = Vec::new();
    let linked_files = vec![
        Arc::new(model_file_with_db_field(
            "AgentRun",
            "runtimeBindings",
            service_symbol_type("tools", "AgentRuntimeBindings"),
        )),
        Arc::new(empty_file("runner")),
    ];
    let packages = vec![crate::test_support::runtime_execution_package_fixture(
        "skiff.test/db-model",
        0,
        linked_files.clone(),
        Default::default(),
    )];
    let link_overlay = Default::default();
    let current_addr = ExecutableAddr::package(0, 1, 0);
    let ctx = PlanContext::from_type_view(
        ProgramTypeView::new(&service_files, &packages, &link_overlay, &types),
        &current_addr,
    );
    let plans = DbIrEvaluator::db_declaration_recoverable_expected_plans(
        &linked_files[0].declarations.db["AgentRun"],
        &ctx,
    )
    .expect("package DB declaration field plans");

    assert!(expected_contains_any_interface(
        plans
            .field("runtimeBindings")
            .expect("runtimeBindings plan")
    ));
    assert!(plans.fields().values().any(expected_contains_any_interface));
}

#[test]
fn service_db_object_symbol_declaration_in_another_file_generates_runtime_plans() {
    let mut types = RuntimeTypeContext::default();
    register_exported_record_with_any_interface(
        &mut types,
        "tools",
        "AgentRuntimeBindings",
        0,
        BTreeMap::from([(
            "events".to_string(),
            any_interface("events.AgentEventReceiver"),
        )]),
    );

    let service_files = vec![
        Arc::new(model_file_with_db_field(
            "AgentRun",
            "runtimeBindings",
            service_symbol_type("tools", "AgentRuntimeBindings"),
        )),
        Arc::new(empty_file("thread")),
    ];
    let packages: Vec<Arc<RuntimeExecutionPackage>> = Vec::new();
    let link_overlay = Default::default();
    let current_addr = ExecutableAddr::service(1, 0);
    let ctx = PlanContext::from_type_view(
        ProgramTypeView::new(&service_files, &packages, &link_overlay, &types),
        &current_addr,
    );
    let plans = DbIrEvaluator::db_declaration_recoverable_expected_plans(
        &service_files[0].declarations.db["AgentRun"],
        &ctx,
    )
    .expect("service DB declaration field plans");

    assert!(expected_contains_any_interface(
        plans
            .field("runtimeBindings")
            .expect("runtimeBindings plan")
    ));
}

#[test]
fn nested_path_on_recoverable_top_level_field_uses_top_level_plan() {
    let mut types = RuntimeTypeContext::default();
    register_exported_record_with_any_interface(
        &mut types,
        "tools",
        "AgentThreadConfig",
        0,
        BTreeMap::from([(
            "runtimeBindings".to_string(),
            service_symbol_type("tools", "AgentRuntimeBindings"),
        )]),
    );
    register_exported_record_with_any_interface(
        &mut types,
        "tools",
        "AgentRuntimeBindings",
        1,
        BTreeMap::from([(
            "events".to_string(),
            any_interface("events.AgentEventReceiver"),
        )]),
    );

    let service_files = vec![
        Arc::new(model_file_with_db_field(
            "AgentThread",
            "currentConfig",
            service_symbol_type("tools", "AgentThreadConfig"),
        )),
        Arc::new(empty_file("runner")),
    ];
    let packages: Vec<Arc<RuntimeExecutionPackage>> = Vec::new();
    let link_overlay = Default::default();
    let current_addr = ExecutableAddr::service(1, 0);
    let ctx = PlanContext::from_type_view(
        ProgramTypeView::new(&service_files, &packages, &link_overlay, &types),
        &current_addr,
    );
    let plans = DbIrEvaluator::db_declaration_recoverable_expected_plans(
        &service_files[0].declarations.db["AgentThread"],
        &ctx,
    )
    .expect("nested recoverable field plans");

    assert!(
        recoverable_plan_for_field_path(&plans, "currentConfig.runtimeBindings")
            .is_some_and(expected_contains_any_interface)
    );
}

#[test]
fn plain_cross_file_db_object_symbol_does_not_require_runtime_plan() {
    let types = RuntimeTypeContext::default();
    let service_files = vec![
        Arc::new(model_file_with_db_field("AgentRun", "title", string_type())),
        Arc::new(empty_file("runner")),
    ];
    let packages: Vec<Arc<RuntimeExecutionPackage>> = Vec::new();
    let link_overlay = Default::default();
    let current_addr = ExecutableAddr::service(1, 0);
    let ctx = PlanContext::from_type_view(
        ProgramTypeView::new(&service_files, &packages, &link_overlay, &types),
        &current_addr,
    );
    let plans = DbIrEvaluator::db_declaration_recoverable_expected_plans(
        &service_files[0].declarations.db["AgentRun"],
        &ctx,
    )
    .expect("plain DB declaration field plans");

    assert!(!plans.fields().values().any(expected_contains_any_interface));
}

fn register_exported_record_with_any_interface(
    types: &mut RuntimeTypeContext,
    module_path: &str,
    symbol: &str,
    type_index: usize,
    fields: BTreeMap<String, LinkedTypeRef>,
) {
    let addr = TypeAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(0),
        type_index,
    };
    types.descriptors.insert(
        addr.clone(),
        anonymous_type_decl(symbol, LinkedTypeDescriptor::Record { fields }),
    );
    types
        .exported_types
        .insert_service(ServiceSymbolKey::new(module_path, symbol), addr);
}

fn model_file_with_db_field(
    db_symbol: &str,
    field_name: &str,
    field_ty: LinkedTypeRef,
) -> LinkedFileUnit {
    let mut file = empty_file("model");
    file.declarations.db.insert(
        db_symbol.to_string(),
        DbDeclarationIr {
            type_ref: db_object_type("model", db_symbol),
            type_name: format!("model.{db_symbol}"),
            collection_name: Some(db_symbol.to_string()),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: string_type(),
            },
            fields: vec![DbObjectFieldIr {
                name: field_name.to_string(),
                ty: field_ty,
                storage: Default::default(),
            }],
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    file
}

fn empty_file(module_path: &str) -> LinkedFileUnit {
    LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: format!("file:{module_path}"),
        source_ast_hash: format!("source:{module_path}"),
        module_path: module_path.to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: Default::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: Vec::new(),
        constants: Vec::new(),
        executables: Vec::new(),
        external_refs: Default::default(),
    }
}

fn db_object_type(module_path: &str, symbol: &str) -> LinkedTypeRef {
    LinkedTypeRef::DbObjectSymbol {
        symbol: service_symbol(module_path, symbol),
    }
}

fn service_symbol_type(module_path: &str, symbol: &str) -> LinkedTypeRef {
    LinkedTypeRef::ServiceSymbol {
        symbol: service_symbol(module_path, symbol),
    }
}

fn service_symbol(module_path: &str, symbol: &str) -> ServiceSymbolRef {
    ServiceSymbolRef {
        module_path: module_path.to_string(),
        symbol: symbol.to_string(),
    }
}

fn any_interface(interface_abi_id: &str) -> LinkedTypeRef {
    LinkedTypeRef::AnyInterface {
        interface: LinkedInterfaceInstantiationRef {
            interface_abi_id: interface_abi_id.to_string(),
            canonical_type_args: Vec::new(),
        },
    }
}

fn string_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "String".to_string(),
        args: Vec::new(),
    }
}
