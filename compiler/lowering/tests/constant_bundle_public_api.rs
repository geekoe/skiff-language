use skiff_compiler_lowering::{
    Bounds, ConstEvaluator, ConstEvaluatorError, FrozenConstantBundle, FrozenConstantLookupError,
    FrozenConstantShape,
};

#[test]
fn constant_bundle_contract_is_reachable_from_the_crate_root() {
    fn assert_public_type<T>() {}

    assert_public_type::<Bounds>();
    assert_public_type::<ConstEvaluator>();
    assert_public_type::<ConstEvaluatorError>();
    assert_public_type::<FrozenConstantBundle>();
    assert_public_type::<FrozenConstantLookupError>();
    assert_public_type::<FrozenConstantShape>();

    let _: fn(
        &ConstEvaluator,
        &skiff_artifact_model::FileIrUnit,
    ) -> Result<FrozenConstantBundle, ConstEvaluatorError> = ConstEvaluator::evaluate_unit;
}
