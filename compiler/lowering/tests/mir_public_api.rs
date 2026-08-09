use skiff_compiler_lowering::mir::{
    builder::build_mir_units, liveness::compute_liveness, MirBuildError, MirCallWritableFacts,
    MirConst, MirContractError, MirExpression, MirForInBinding, MirForInFacts, MirForInItemKind,
    MirFunction, MirInOutLoan, MirLiveness, MirUnit, MirWritablePathSegment, MirWritablePlace,
    MirWritableRoot,
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
}
