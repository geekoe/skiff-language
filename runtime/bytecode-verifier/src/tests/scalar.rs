use skiff_runtime_linked_bytecode::ConstantIndex;
use skiff_runtime_model::vm_value::{CompactTypeTag, ValueFlags, ValueSlot, VmHandle};

use super::fixtures::{generous_limits, loader_backed_scalar};
use crate::verify_facts;

#[test]
fn public_verify_accepts_scalar_function_and_materializes_literal_heap() {
    let fixture = loader_backed_scalar();
    let image = verify_facts(fixture.hydrated, fixture.candidate, &generous_limits())
        .expect("scalar opcodes and literal constants verify through the public entry");

    let heap = image.constant_heap();
    assert_eq!(heap.len(), 5);
    assert!(heap.get(ConstantIndex::new(0)) == Some(ValueSlot::null()));
    assert!(heap.get(ConstantIndex::new(1)) == Some(ValueSlot::bool(true)));
    assert!(heap.get(ConstantIndex::new(2)) == Some(ValueSlot::number(2.5)));
    assert!(heap.get(ConstantIndex::new(3)) == Some(ValueSlot::number(3.5)));
    assert!(
        heap.get(ConstantIndex::new(4))
            == Some(ValueSlot::const_ref(
                VmHandle::new(4),
                CompactTypeTag::new(3),
                ValueFlags::new(0)
            ))
    );
}
