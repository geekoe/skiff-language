use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{RuntimeMap, RuntimeValue},
};

use crate::eval::error::Result;

use super::RuntimeValueView;

#[test]
fn views_erased_payloads_for_string_array_and_map_values() -> Result<()> {
    let mut heap = RequestHeap::default();
    let string = RuntimeValue::String("abc".to_string());
    assert_eq!(
        RuntimeValueView::new(&string, &heap).string_payload()?,
        Some("abc")
    );

    let array = RuntimeValue::Heap(heap.alloc_array(vec![RuntimeValue::Number(1.0)])?);
    assert_eq!(
        RuntimeValueView::new(&array, &heap)
            .array_items()?
            .as_ref()
            .map(Vec::len),
        Some(1)
    );

    let map = RuntimeValue::Heap(heap.alloc_map(RuntimeMap::new())?);
    assert!(RuntimeValueView::new(&map, &heap).is_map_like()?);
    assert_eq!(RuntimeValueView::new(&map, &heap).map_like_len()?, 0);

    let bytes = RuntimeValue::Heap(heap.alloc_bytes(b"abc".as_slice())?);
    assert_eq!(
        RuntimeValueView::new(&bytes, &heap)
            .bytes_payload()?
            .map(|bytes| bytes.as_slice()),
        Some(b"abc".as_slice())
    );

    Ok(())
}
