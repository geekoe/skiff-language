use skiff_runtime_linked_program::{
    anonymous_type_decl, FileAddr, LinkOverlay, LinkedTypeDescriptor, LinkedTypeRef,
    RuntimeTypeContext, TypeAddr, UnitAddr,
};

use crate::type_plan::test_runtime_package;

use super::*;

#[test]
fn binary_http_response_plan_requires_std_package_nominal_type() {
    let std_response_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let spoof_response_addr = TypeAddr {
        unit: UnitAddr::Package(1),
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let mut types = RuntimeTypeContext::default();
    types.descriptors.insert(
        std_response_addr.clone(),
        anonymous_type_decl(
            "HttpResponse",
            LinkedTypeDescriptor::Record {
                fields: Default::default(),
            },
        ),
    );
    types.descriptors.insert(
        spoof_response_addr.clone(),
        anonymous_type_decl(
            "HttpResponse",
            LinkedTypeDescriptor::Record {
                fields: Default::default(),
            },
        ),
    );
    let packages = vec![
        test_runtime_package(0, "skiff.run/std", Vec::new()),
        test_runtime_package(1, "example.com/std-lookalike", Vec::new()),
    ];
    let overlay = LinkOverlay::default();
    let program = ProgramTypeView::new(&[], &packages, &overlay, &types);
    let current_addr = ExecutableAddr::service(0, 0);

    let std_response_ref = LinkedTypeRef::Address {
        addr: std_response_addr,
    };
    binary_http_response_plan(Some(&std_response_ref), program, &current_addr)
        .expect("std package HttpResponse address should be accepted");

    let spoof_response_ref = LinkedTypeRef::Address {
        addr: spoof_response_addr,
    };
    let error = binary_http_response_plan(Some(&spoof_response_ref), program, &current_addr)
        .expect_err("lookalike package HttpResponse address should be rejected");
    assert!(error.to_string().contains("std.http.HttpResponse"));
}
