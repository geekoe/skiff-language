use std::sync::Arc;

use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, LinkedCallTarget, LinkedExprIr, UnitAddr,
};
use skiff_runtime_loader::RuntimeAssemblyLoader;

use super::{link_runtime_assembly, linked_call, CycleFixture};

fn exact_call_addr(expression: &LinkedExprIr) -> &ExecutableAddr {
    let LinkedExprIr::Call { call } = expression else {
        panic!("fixture expression should remain a call")
    };
    let LinkedCallTarget::Executable { addr } = &call.target else {
        panic!(
            "assembly linker should normalize executable target, found {:?}",
            call.target
        )
    };
    addr
}

#[test]
fn assembly_linker_normalizes_local_and_publication_targets_to_exact_addresses() {
    let fixture = CycleFixture::new();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly)
        .expect("fixture assembly should hydrate");
    let candidate =
        link_runtime_assembly(hydrated).expect("fixture assembly should initially link");
    let mut files = candidate
        .execution_image()
        .execution_packages()
        .iter()
        .map(|code| code.files().to_vec())
        .collect::<Vec<_>>();

    let mut sibling = files[0][0].as_ref().clone();
    sibling.file_ir_identity = "fixture:shared-tail-target".to_string();
    sibling.source_ast_hash = "source:shared-tail-target".to_string();
    sibling.module_path = "shared.tail_target".to_string();
    sibling.declarations.types.clear();
    sibling.declarations.interfaces.clear();
    sibling.declarations.db.clear();
    sibling.declarations.executables.clear();
    sibling.declarations.constants.clear();
    sibling.declarations.symbols.clear();
    sibling.link_targets.types.clear();
    sibling.link_targets.executables.clear();
    sibling.link_targets.constants.clear();
    sibling.actor_declarations.clear();
    sibling.types.clear();
    sibling.constants.clear();
    sibling.executables.truncate(1);
    sibling.executables[0].symbol = "shared.tail_target.hop".to_string();
    sibling.executables[0].body = Default::default();
    files[0].push(Arc::new(sibling));

    let primary = Arc::make_mut(&mut files[0][0]);
    let local_expression = primary.executables[0].body.expressions.len();
    primary.executables[0]
        .body
        .expressions
        .push(LinkedExprIr::Call {
            call: linked_call(
                LinkedCallTarget::LocalExecutable {
                    executable_index: 1,
                },
                0,
            ),
        });
    let publication_expression = primary.executables[0].body.expressions.len();
    primary.executables[0]
        .body
        .expressions
        .push(LinkedExprIr::Call {
            call: linked_call(
                LinkedCallTarget::PublicationExecutable {
                    module_path: "shared.tail_target".to_string(),
                    executable_index: 0,
                },
                0,
            ),
        });

    let linked = crate::assembly_execution::relink_execution_files_for_test(
        candidate.shared_image().as_ref(),
        &files,
    )
    .expect("canonical assembly relinker should resolve both targets");
    let expressions = &linked[0][0].executables[0].body.expressions;

    assert_eq!(
        exact_call_addr(&expressions[local_expression]),
        &ExecutableAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            executable: 1,
        }
    );
    assert_eq!(
        exact_call_addr(&expressions[publication_expression]),
        &ExecutableAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(1),
            executable: 0,
        }
    );
}
