use super::*;

#[test]
fn from_base64_returns_fresh_bytes_and_preserves_typed_decode_error() {
    let mut heap = RequestHeap::default();
    let first = eval_program_bytes_native(
        "core.bytes.fromBase64",
        vec![RuntimeValue::String("aGVsbG8=".to_string())],
        &mut heap,
    )
    .expect("valid Base64 should decode");
    let second = eval_program_bytes_native(
        "core.bytes.fromBase64",
        vec![RuntimeValue::String("aGVsbG8=".to_string())],
        &mut heap,
    )
    .expect("a second decode should succeed");

    let (RuntimeValue::Heap(first_handle), RuntimeValue::Heap(second_handle)) = (first, second)
    else {
        panic!("Base64 decoder should return heap-backed bytes");
    };
    assert_ne!(
        first_handle, second_handle,
        "each call must allocate fresh bytes"
    );
    assert_eq!(
        heap.get(first_handle).expect("first bytes should exist"),
        &HeapNode::Bytes(b"hello".to_vec().into())
    );
    assert_eq!(
        heap.get(second_handle).expect("second bytes should exist"),
        &HeapNode::Bytes(b"hello".to_vec().into())
    );

    let error = eval_program_bytes_native(
        "core.bytes.fromBase64",
        vec![RuntimeValue::String("***not-base64***".to_string())],
        &mut heap,
    )
    .expect_err("invalid Base64 must remain a typed decode error");
    let RuntimeError::BytesDecode { target, .. } = error else {
        panic!("invalid Base64 should preserve RuntimeError::BytesDecode");
    };
    assert_eq!(target, "bytes.fromBase64");
}

#[test]
fn from_hex_returns_fresh_bytes_and_preserves_typed_decode_error() {
    let mut heap = RequestHeap::default();
    let first = eval_program_bytes_native(
        "core.bytes.fromHex",
        vec![RuntimeValue::String("68656c6c6f".to_string())],
        &mut heap,
    )
    .expect("valid hex should decode");
    let second = eval_program_bytes_native(
        "core.bytes.fromHex",
        vec![RuntimeValue::String("68656c6c6f".to_string())],
        &mut heap,
    )
    .expect("a second decode should succeed");

    let (RuntimeValue::Heap(first_handle), RuntimeValue::Heap(second_handle)) = (first, second)
    else {
        panic!("hex decoder should return heap-backed bytes");
    };
    assert_ne!(
        first_handle, second_handle,
        "each call must allocate fresh bytes"
    );
    assert_eq!(
        heap.get(first_handle).expect("first bytes should exist"),
        &HeapNode::Bytes(b"hello".to_vec().into())
    );
    assert_eq!(
        heap.get(second_handle).expect("second bytes should exist"),
        &HeapNode::Bytes(b"hello".to_vec().into())
    );

    let error = eval_program_bytes_native(
        "core.bytes.fromHex",
        vec![RuntimeValue::String("not-hex".to_string())],
        &mut heap,
    )
    .expect_err("invalid hex must remain a typed decode error");
    let RuntimeError::BytesDecode { target, .. } = error else {
        panic!("invalid hex should preserve RuntimeError::BytesDecode");
    };
    assert_eq!(target, "bytes.fromHex");
}
