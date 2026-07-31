use std::sync::Arc;

use super::*;
use skiff_artifact_model::{SourcePosition, SourceSpanRef};
use skiff_runtime_linked_program::{
    LinkOverlay, RuntimeExecutionPackage, RuntimeTypeContext, TypeDeclIr,
};
use skiff_runtime_model::runtime_value::{HeapNode, RuntimeValue};

struct TestProgramTypeView {
    service_files: Vec<Arc<LinkedFileUnit>>,
    packages: Vec<Arc<RuntimeExecutionPackage>>,
    link_overlay: LinkOverlay,
    types: RuntimeTypeContext,
}

impl TestProgramTypeView {
    fn empty() -> Self {
        Self {
            service_files: Vec::new(),
            packages: Vec::new(),
            link_overlay: LinkOverlay::default(),
            types: RuntimeTypeContext::default(),
        }
    }

    fn view(&self) -> ProgramTypeView<'_> {
        ProgramTypeView::new(
            &self.service_files,
            &self.packages,
            &self.link_overlay,
            &self.types,
        )
    }
}

fn addr(type_index: usize) -> TypeAddr {
    TypeAddr {
        unit: UnitAddr::Service,
        file: FileAddr::loaded_file(0),
        type_index,
    }
}

fn type_decl(name: &str, descriptor: LinkedTypeDescriptor, type_params: Vec<&str>) -> TypeDeclIr {
    TypeDeclIr {
        name: name.to_string(),
        descriptor,
        type_params: type_params.into_iter().map(str::to_string).collect(),
        implements: Vec::new(),
        source_span: None,
    }
}

fn source_site() -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 9,
            start: SourcePosition::new(4, 2),
            end: SourcePosition::new(4, 7),
        },
    }
}

fn local_exception(identity: CatchIdentity) -> RequestException {
    RequestException::local(
        RuntimeValueCarrier::identified(RuntimeValue::from("denied"), identity),
        source_site(),
        vec![ExceptionStackFrame::Local {
            site: source_site(),
        }],
        ErrorCorrelation {
            trace_id: "trace-local".to_string(),
            error_id: "trace-local:local-error:1".to_string(),
        },
    )
    .expect("local exception")
}

#[test]
fn local_catch_and_rethrow_preserve_the_exact_request_exception() {
    let identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: addr(3),
            type_arguments: Vec::new(),
        },
    ));
    let exception = local_exception(identity.clone());
    let error = RuntimeError::UserException(UserException::new(exception.clone()))
        .with_diagnostic_frame(serde_json::json!({ "sourceId": 7 }));
    let mut heap = RequestHeap::default();

    let caught = request_exception_for_catch(
        &error,
        &[identity],
        source_site(),
        vec![ExceptionStackFrame::Local {
            site: source_site(),
        }],
        ErrorCorrelation {
            trace_id: "unused".to_string(),
            error_id: "unused".to_string(),
        },
        &mut heap,
    )
    .expect("catch projection")
    .expect("matching exception");
    assert_eq!(caught, exception);

    let caught_value = catch_err(caught, &mut heap).expect("catch result");
    let RuntimeValue::Heap(catch_handle) = caught_value.value() else {
        panic!("catch result must be a request-local object");
    };
    let exception_value = heap
        .object_field_carrier(*catch_handle, "exception")
        .expect("catch object")
        .expect("exception field");
    let HeapNode::Exception(stored) = heap
        .get(match exception_value.value() {
            RuntimeValue::Heap(handle) => *handle,
            _ => panic!("exception field must be a heap handle"),
        })
        .expect("exception node")
    else {
        panic!("exception field must retain RequestException");
    };
    assert_eq!(stored, &exception);
    assert_eq!(
        request_exception_for_rethrow(&exception_value, &heap).expect("rethrow"),
        exception
    );
}

#[test]
fn fully_instantiated_generic_identities_are_exact_and_fail_closed() {
    let generic_addr = addr(0);
    let mut program = TestProgramTypeView::empty();
    program.types.descriptors.insert(
        generic_addr.clone(),
        type_decl(
            "Failure",
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
            vec!["T"],
        ),
    );
    let applied = |name: &str| LinkedTypeRef::AppliedNominal {
        base: LinkedNominalTypeRefBase::Address {
            addr: generic_addr.clone(),
        },
        arguments: vec![LinkedTypeRef::Native {
            name: name.to_string(),
            args: Vec::new(),
        }],
    };
    let string_ref = applied("string");
    let bool_ref = applied("bool");
    let string_identity =
        catch_type_leaves(&string_ref, program.view()).expect("string instantiation")[0].clone();
    let bool_identity =
        catch_type_leaves(&bool_ref, program.view()).expect("bool instantiation")[0].clone();

    assert_ne!(string_identity, bool_identity);
    let mut plan = RuntimeTypePlan::synthetic_request_record(Vec::new());
    annotate_runtime_type_plan(&mut plan, &string_ref, program.view())
        .expect("generic plan annotation");
    assert_eq!(plan.catch_identity(), Some(&string_identity));
    assert!(catch_type_leaves(
        &LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::Address { addr: generic_addr },
            arguments: Vec::new(),
        },
        program.view(),
    )
    .is_err());
    assert!(catch_type_leaves(
        &LinkedTypeRef::TypeParam {
            name: "T".to_string(),
        },
        program.view(),
    )
    .is_err());
}

#[test]
fn aliases_expand_transparently_and_named_union_owners_remain_distinct() {
    let record_addr = addr(1);
    let alias_addr = addr(2);
    let first_union_addr = addr(3);
    let second_union_addr = addr(4);
    let branch = LinkedNamedUnionBranch::SyntheticDiscriminator {
        payload_type: LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        },
        discriminator_field: "kind".to_string(),
        discriminator_value: "denied".to_string(),
    };
    let mut program = TestProgramTypeView::empty();
    program.types.descriptors.insert(
        record_addr.clone(),
        type_decl(
            "Failure",
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
            Vec::new(),
        ),
    );
    program.types.descriptors.insert(
        alias_addr.clone(),
        type_decl(
            "FailureAlias",
            LinkedTypeDescriptor::Alias {
                target: LinkedTypeRef::Address {
                    addr: record_addr.clone(),
                },
            },
            Vec::new(),
        ),
    );
    for (union_addr, name) in [
        (first_union_addr.clone(), "FirstFailure"),
        (second_union_addr.clone(), "SecondFailure"),
    ] {
        program.types.descriptors.insert(
            union_addr,
            type_decl(
                name,
                LinkedTypeDescriptor::Union {
                    branches: vec![branch.clone()],
                },
                Vec::new(),
            ),
        );
    }

    assert_eq!(
        catch_type_leaves(&LinkedTypeRef::Address { addr: alias_addr }, program.view(),)
            .expect("alias leaves"),
        catch_type_leaves(
            &LinkedTypeRef::Address { addr: record_addr },
            program.view(),
        )
        .expect("record leaves")
    );
    let first = catch_type_leaves(
        &LinkedTypeRef::Address {
            addr: first_union_addr,
        },
        program.view(),
    );
    let second = catch_type_leaves(
        &LinkedTypeRef::Address {
            addr: second_union_addr,
        },
        program.view(),
    );
    assert_ne!(first.expect("first union"), second.expect("second union"));
}

#[test]
fn finite_platform_error_registry_promotes_to_a_local_exception() {
    assert_eq!(
        PlatformBuiltinErrorIdentity::from_symbol("std.resource.ResourceError"),
        None
    );
    let identity = PlatformBuiltinErrorIdentity::DbDecode.catch_identity();
    let mut heap = RequestHeap::default();
    let exception = request_exception_for_catch(
        &RuntimeError::DbDecode {
            target: "std.db".to_string(),
            message: "missing id".to_string(),
        },
        std::slice::from_ref(&identity),
        source_site(),
        vec![ExceptionStackFrame::Local {
            site: source_site(),
        }],
        ErrorCorrelation {
            trace_id: "trace-platform".to_string(),
            error_id: "trace-platform:local-error:1".to_string(),
        },
        &mut heap,
    )
    .expect("platform promotion")
    .expect("matching platform exception");

    assert_eq!(exception.local_catch_identity(), Some(&identity));
    assert_eq!(exception.source(), &source_site());
    assert!(exception.local_value().is_some());
}

#[test]
fn cancellation_terminal_cannot_materialize_a_request_exception() {
    let mut heap = RequestHeap::default();
    let leaves = [PlatformBuiltinErrorIdentity::Timeout.catch_identity()];
    let exception = request_exception_for_catch(
        &RuntimeError::Cancelled.with_diagnostic_frame(serde_json::json!({
            "sourceId": 7,
        })),
        &leaves,
        source_site(),
        vec![ExceptionStackFrame::Local {
            site: source_site(),
        }],
        ErrorCorrelation {
            trace_id: "trace-cancel".to_string(),
            error_id: "trace-cancel:local-error:1".to_string(),
        },
        &mut heap,
    )
    .expect("cancellation classification must not fail");

    assert_eq!(exception, None);
}
