use std::collections::BTreeSet;

use skiff_artifact_model::{
    bytecode::{BytecodeFunctionOrigin, BytecodeRelocation, ValueDropPlan, ValueTransferPlan},
    BytecodePoolEntry, HostEffectExecutorIdentity, Opcode, PackageExecutableCoordinate,
    PrivilegedAffineCompositeIdentity, ResourceDropPlan,
};
use skiff_runtime_bytecode_verifier::VerifiedResumeKind;
use skiff_runtime_linked_bytecode::{
    InstructionIndex, LinkedInstructionTarget, LinkedResourceDropPlan, LinkedValueDropPlan,
    LinkedValueTransferPlan,
};

use super::fixture::{BuildOutcome, FixtureSpec, PublishedFixture};

const VCP_PATH: &str = "/phase-5/vcp";
const DROP_PATH: &str = "/phase-5/drop-left";

pub fn published_positive(prefix: &str) -> PublishedFixture {
    match FixtureSpec::positive().build(prefix) {
        BuildOutcome::Published(fixture) => fixture,
        BuildOutcome::Rejected { error_chain, .. } => panic!(
            "the production source/publication carrier did not reach this stage: {error_chain}"
        ),
    }
}

#[test]
fn phase_5_stage_sentinel_admission_to_emission() {
    let fixture = published_positive("s2-emission");
    let bytecode = fixture.bytecode();
    let package = fixture.package_artifact();
    let deployment = fixture.deployment_artifact();
    let function = gateway_artifact_function(&package, &deployment, bytecode.view(), VCP_PATH);
    let drop_left_function =
        gateway_artifact_function(&package, &deployment, bytecode.view(), DROP_PATH);

    for (handler, emitted) in [("run", function), ("dropLeft", drop_left_function)] {
        assert_eq!(
            emitted
                .instructions
                .iter()
                .filter(|instruction| instruction.descriptor.kind == Opcode::NewArrayBuilder)
                .count(),
            1,
            "{handler} response.start headers use the typed empty-array builder"
        );
        assert_eq!(
            emitted
                .instructions
                .iter()
                .filter(|instruction| instruction.descriptor.kind == Opcode::FreezeArray)
                .count(),
            1,
            "{handler} response.start headers freeze the typed empty array"
        );
    }
    assert_eq!(
        bytecode
            .view()
            .functions()
            .iter()
            .flat_map(|function| function.relocations.iter())
            .filter(|relocation| {
                matches!(
                    relocation,
                    BytecodeRelocation::IntrinsicRef { intrinsic }
                        if matches!(
                            &intrinsic.target,
                            skiff_artifact_model::BytecodeIntrinsicRef::Static { .. }
                        )
                )
            })
            .count(),
        0,
        "the Phase 5 carrier must not regain Array.empty through a static intrinsic relocation"
    );

    let host_bindings = function
        .instructions
        .iter()
        .filter(|instruction| instruction.descriptor.kind == Opcode::InvokeHost)
        .map(|instruction| {
            let relocation = function
                .relocations
                .get(instruction.operand(0) as usize)
                .expect("InvokeHost relocation index");
            let BytecodeRelocation::HostEffectRef(effect) = relocation else {
                panic!("InvokeHost must retain a typed HostEffectRef relocation")
            };
            effect
                .target
                .binding_key
                .as_deref()
                .expect("HostEffectRef exact canonical binding")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        host_bindings,
        [
            "std.http.client.request",
            "std.http.client.stream",
            "std.http.client.stream",
        ],
        "the exact VCP gateway must carry one request and two stream callsites"
    );

    let body_takes = function
        .instructions
        .windows(2)
        .filter(|pair| {
            pair[0].descriptor.kind == Opcode::TakeSlot
                && pair[1].descriptor.kind == Opcode::TakeDenseField
        })
        .map(|pair| &pair[1])
        .collect::<Vec<_>>();
    assert_eq!(
        body_takes.len(),
        2,
        "left.body and right.body need two consume-whole affine takes"
    );
    for take in &body_takes {
        assert_eq!(take.operand(1), 0, "body is exact dense ordinal zero");
        let shape_ref = take.operand(0);
        let Some(BytecodePoolEntry::ShapeRef { shape }) =
            bytecode.view().pools().shapes.get(shape_ref as usize)
        else {
            panic!("TakeDenseField does not retain a typed shape declaration")
        };
        assert_eq!(
            shape.privileged_affine_composite,
            Some(PrivilegedAffineCompositeIdentity::HttpClientStreamHandle)
        );
        assert_eq!(
            shape
                .fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            ["body", "headers", "status"]
        );
        assert_eq!(
            shape.fields[0].plan,
            ValueTransferPlan::AffineResource {
                drop: ResourceDropPlan::ResourceTableRelease,
            }
        );
        assert!(function.instructions.iter().all(|instruction| {
            instruction.descriptor.kind != Opcode::GetDenseField
                || instruction.operand(0) != shape_ref
                || instruction.operand(1) != 0
        }));
    }

    let stream_next_sites = function
        .instructions
        .iter()
        .filter(|instruction| instruction.descriptor.kind == Opcode::StreamNext)
        .collect::<Vec<_>>();
    assert_eq!(
        stream_next_sites.len(),
        2,
        "A and B need independent StreamNext sites"
    );
    for instruction in stream_next_sites {
        let resume_index = instruction.operand(1);
        let resume = bytecode
            .view()
            .resume_sites()
            .iter()
            .find(|resume| {
                resume.function_key == function.function_key
                    && resume.descriptor_index == resume_index
            })
            .expect("StreamNext carries an admitted resume descriptor");
        assert!(
            resume.end_resume_pc.is_some(),
            "StreamNext certifies natural end"
        );
    }

    let recursive_move_slots = function
        .frame_layout
        .slot_plans
        .iter()
        .filter(|plan| {
            matches!(
                plan,
                ValueTransferPlan::MoveOnly {
                    drop: ValueDropPlan::RecursiveShape { .. }
                }
            )
        })
        .count();
    assert_eq!(
        recursive_move_slots, 2,
        "the two HTTP stream handle locals require recursive remainder-drop plans"
    );
}

#[test]
fn phase_5_stage_sentinel_emission_to_link() {
    let fixture = published_positive("s3-link");
    let gateway = fixture.gateway(VCP_PATH);
    let image = fixture.link();
    let entry = image
        .http_gateway_entry(&gateway.ingress, &gateway.identity)
        .expect("production linker resolves the exact VCP gateway");
    let function = &image.functions()[entry.function().get() as usize];

    let executor_identities = function
        .instructions()
        .iter()
        .filter(|instruction| instruction.opcode() == Opcode::InvokeHost)
        .map(|instruction| {
            let index = instruction
                .resolved_operands()
                .iter()
                .find_map(|operand| match operand.target() {
                    LinkedInstructionTarget::HostEffectAdapter(index) => Some(index),
                    _ => None,
                })
                .expect("InvokeHost retains an exact typed host target index");
            image
                .host_effect_target(index)
                .expect("typed host target index resolves in the production image")
                .executor_identity()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        executor_identities,
        [
            HostEffectExecutorIdentity::HttpClientRequest,
            HostEffectExecutorIdentity::HttpClientStream,
            HostEffectExecutorIdentity::HttpClientStream,
        ],
        "the linked image must expose only registry-owned executor identities"
    );
    assert_eq!(
        executor_identities
            .into_iter()
            .collect::<BTreeSet<_>>()
            .len(),
        2,
        "two stream calls share one exact typed target while request remains distinct"
    );
}

#[test]
fn phase_5_stage_sentinel_link_to_verify() {
    let fixture = published_positive("s4-verify");
    let gateway = fixture.gateway(VCP_PATH);
    let image = fixture.link();
    let entry = image
        .http_gateway_entry(&gateway.ingress, &gateway.identity)
        .expect("verified image resolves exact VCP gateway");
    let function_index = entry.function();
    let function = &image.functions()[function_index.get() as usize];
    let mut host_identities = Vec::new();
    for (ordinal, instruction) in function.instructions().iter().enumerate() {
        if instruction.opcode() != Opcode::InvokeHost {
            continue;
        }
        let target_index = instruction
            .resolved_operands()
            .iter()
            .find_map(|operand| match operand.target() {
                LinkedInstructionTarget::HostEffectAdapter(index) => Some(index),
                _ => None,
            })
            .expect("verified InvokeHost retains its typed target index");
        let resume_index = instruction
            .resolved_operands()
            .iter()
            .find_map(|operand| match operand.target() {
                LinkedInstructionTarget::ResumeSite(index) => Some(index),
                _ => None,
            })
            .expect("verified InvokeHost retains its typed resume index");
        let target = image
            .host_effect_target(target_index)
            .expect("verified target remains accessible only by its typed index");
        let resume = image
            .resume_sites()
            .get(resume_index)
            .expect("verified image retains the exact host resume certificate");
        assert_eq!(resume.function(), function_index);
        assert_eq!(
            resume.site(),
            InstructionIndex::new(u32::try_from(ordinal).expect("instruction ordinal fits u32"))
        );
        assert_eq!(resume.kind(), &VerifiedResumeKind::HostEffect);
        assert_eq!(resume.result_types(), target.signature().result_types());
        assert_eq!(resume.result_plans(), target.signature().result_plans());
        host_identities.push(target.executor_identity());
    }
    assert_eq!(
        host_identities,
        [
            HostEffectExecutorIdentity::HttpClientRequest,
            HostEffectExecutorIdentity::HttpClientStream,
            HostEffectExecutorIdentity::HttpClientStream,
        ]
    );

    let sites = image
        .resume_sites()
        .rows()
        .iter()
        .filter(|site| site.function() == function_index)
        .collect::<Vec<_>>();
    assert_eq!(
        sites
            .iter()
            .filter(|site| matches!(site.kind(), VerifiedResumeKind::HostEffect))
            .count(),
        3,
        "verifier certifies every HTTP host call resume"
    );
    let stream_reads = sites
        .iter()
        .filter(|site| matches!(site.kind(), VerifiedResumeKind::StreamRead { .. }))
        .collect::<Vec<_>>();
    assert_eq!(stream_reads.len(), 2, "verifier certifies A/B item resumes");
    assert!(
        stream_reads.iter().all(|site| site.end_resume().is_some()),
        "verifier certifies the distinct natural-end continuation"
    );
    assert!(
        sites
            .iter()
            .any(|site| matches!(site.kind(), VerifiedResumeKind::StreamBackpressure)),
        "serverStream emit must retain an actual backpressure resume certificate"
    );

    let privileged_shape = image
        .shapes()
        .iter()
        .find(|shape| {
            shape.privileged_affine_composite()
                == Some(PrivilegedAffineCompositeIdentity::HttpClientStreamHandle)
        })
        .expect("linked image retains the registry-owned privileged stream shape");
    assert_eq!(
        privileged_shape
            .fields()
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        ["body", "headers", "status"]
    );
    assert!(matches!(
        privileged_shape.fields()[0].plan(),
        LinkedValueTransferPlan::AffineResource {
            drop: LinkedResourceDropPlan::ResourceTableRelease,
        }
    ));

    let body_takes = function
        .instructions()
        .iter()
        .enumerate()
        .filter(|(_, instruction)| instruction.opcode() == Opcode::TakeDenseField)
        .collect::<Vec<_>>();
    assert_eq!(
        body_takes.len(),
        2,
        "verified code retains both affine body takes"
    );
    for (ordinal, take) in body_takes {
        assert_eq!(take.operands()[1], 0, "body is exact dense ordinal zero");
        assert!(take.resolved_operands().iter().any(|operand| {
            operand.target() == LinkedInstructionTarget::Shape(privileged_shape.index())
        }));
        let [root] = function.stack_map().entries()[ordinal].stack_before() else {
            panic!("TakeDenseField must consume exactly one aggregate root")
        };
        assert_eq!(root.ty(), privileged_shape.nominal_type());
        assert!(matches!(
            root.plan(),
            LinkedValueTransferPlan::MoveOnly {
                drop: LinkedValueDropPlan::RecursiveShape { shape },
            } if *shape == privileged_shape.index()
        ));
        let body = function.stack_map().entries()[ordinal + 1]
            .stack_before()
            .last()
            .expect("TakeDenseField produces the exact affine body value");
        assert_eq!(body.ty(), privileged_shape.fields()[0].ty());
        assert_eq!(body.plan(), privileged_shape.fields()[0].plan());
    }
    assert!(
        function.instructions().iter().all(|instruction| {
            instruction.opcode() != Opcode::GetDenseField
                || !instruction.resolved_operands().iter().any(|operand| {
                    operand.target() == LinkedInstructionTarget::Shape(privileged_shape.index())
                })
        }),
        "verified production code must not GetDenseField any body, headers, or status ordinal from the privileged stream shape"
    );
    assert_eq!(
        function
            .frame()
            .slot_plans()
            .iter()
            .filter(|plan| matches!(
                plan,
                LinkedValueTransferPlan::MoveOnly {
                    drop: LinkedValueDropPlan::RecursiveShape { shape },
                } if *shape == privileged_shape.index()
            ))
            .count(),
        2,
        "linked verifier input retains two exact recursive remainder-drop plans"
    );
}

fn gateway_artifact_function<'a>(
    package: &skiff_artifact_model::PackageArtifact,
    deployment: &skiff_artifact_model::ServiceDeployment,
    view: &'a skiff_artifact_model::StructurallyValidatedView,
    path: &str,
) -> &'a skiff_artifact_model::ValidatedFunction {
    let ingress = deployment
        .ingress
        .iter()
        .find(|binding| binding.selector.path == path)
        .expect("exact VCP ingress");
    let handler = deployment.gateway_entries[&ingress.gateway_entry_key]
        .handler
        .as_ref()
        .expect("VCP gateway handler");
    let target = &package.callable_links[handler].target;
    let coordinate = PackageExecutableCoordinate {
        file_ir_identity: target.file_ref.file_ir_identity.clone(),
        module_path: target.file_ref.module_path.clone(),
        executable_index: target.executable_index,
    };
    view.functions()
        .iter()
        .find(|function| {
            matches!(
                &function.origin,
                BytecodeFunctionOrigin::Executable { executable } if executable == &coordinate
            )
        })
        .expect("artifact contains the exact deployment gateway executable")
}
