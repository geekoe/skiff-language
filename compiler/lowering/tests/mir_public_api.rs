use skiff_compiler_lowering::mir::{
    builder::{
        build_mir_units, build_mir_units_with_call_facts, build_mir_units_with_source_facts,
    },
    liveness::compute_liveness,
    MirBuildError, MirCallArgument, MirCallWritableFacts, MirConst, MirContractError,
    MirDirectCallFacts, MirExpression, MirForInBinding, MirForInFacts, MirForInItemKind,
    MirFunction, MirInOutLoan, MirInOutPathSegment, MirIndexAccessFacts, MirIndexPolicy,
    MirIndexReceiverKind, MirLiveness, MirReceiverFacts, MirSourceFacts, MirUnit,
    MirWritablePathSegment, MirWritablePlace, MirWritableRoot,
};

#[test]
fn mir_contract_is_reachable_from_the_crate_root() {
    fn assert_public_type<T>() {}

    assert_public_type::<MirUnit>();
    assert_public_type::<MirFunction>();
    assert_public_type::<MirConst>();
    assert_public_type::<MirExpression>();
    assert_public_type::<MirWritablePlace>();
    assert_public_type::<MirWritableRoot>();
    assert_public_type::<MirWritablePathSegment>();
    assert_public_type::<MirCallWritableFacts>();
    assert_public_type::<MirInOutLoan>();
    assert_public_type::<MirInOutPathSegment>();
    assert_public_type::<MirDirectCallFacts>();
    assert_public_type::<MirCallArgument>();
    assert_public_type::<MirReceiverFacts>();
    assert_public_type::<MirIndexAccessFacts>();
    assert_public_type::<MirIndexPolicy>();
    assert_public_type::<MirIndexReceiverKind>();
    assert_public_type::<MirSourceFacts>();
    assert_public_type::<MirForInFacts>();
    assert_public_type::<MirForInBinding>();
    assert_public_type::<MirForInItemKind>();
    assert_public_type::<MirBuildError>();
    assert_public_type::<MirContractError>();

    let _: fn(&MirFunction) -> Result<MirLiveness, MirContractError> = compute_liveness;
    let _: fn(
        &str,
        &[skiff_artifact_model::FileIrUnit],
        &skiff_compiler_source::SourceCallableEffectFacts,
    ) -> Result<Vec<MirUnit>, MirBuildError> = build_mir_units;
    let _: fn(
        &str,
        &[skiff_artifact_model::FileIrUnit],
        &skiff_compiler_source::SourceCallableEffectFacts,
        &skiff_compiler_source::ResolvedCallTargetFacts,
    ) -> Result<Vec<MirUnit>, MirBuildError> = build_mir_units_with_call_facts;
    let _: fn(
        &str,
        &[skiff_artifact_model::FileIrUnit],
        &skiff_compiler_source::SourceCallableEffectFacts,
        &skiff_compiler_source::ResolvedCallTargetFacts,
        &MirSourceFacts,
    ) -> Result<Vec<MirUnit>, MirBuildError> = build_mir_units_with_source_facts;
}
