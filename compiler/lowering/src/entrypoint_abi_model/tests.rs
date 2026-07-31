use super::*;
use crate::file_ir::{ServiceSymbolRef, TypeRefIr};
use skiff_artifact_identity::interface_instantiation_ref;

#[test]
fn any_interface_source_text_renders_structured_interface_identity_as_type_syntax() {
    let ty = TypeRefIr::AnyInterface {
        interface: interface_instantiation_ref(
            TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "tools".to_string(),
                    symbol: "ToolProvider".to_string(),
                },
            },
            Vec::new(),
        ),
    };

    assert_eq!(
        type_ref_ir_source_text_with_local_types(&ty, &|_| None),
        "any root.tools.ToolProvider"
    );
}

#[test]
fn any_interface_source_text_preserves_canonical_type_arguments() {
    let ty = TypeRefIr::AnyInterface {
        interface: interface_instantiation_ref(
            TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "tools".to_string(),
                    symbol: "Provider".to_string(),
                },
            },
            vec![TypeRefIr::builtin("string")],
        ),
    };

    assert_eq!(
        type_ref_ir_source_text_with_local_types(&ty, &|_| None),
        "any root.tools.Provider<string>"
    );
    assert_eq!(
        crate::type_lowering::type_ref_ir_type_text(&ty),
        "any tools.Provider<string>"
    );
}
