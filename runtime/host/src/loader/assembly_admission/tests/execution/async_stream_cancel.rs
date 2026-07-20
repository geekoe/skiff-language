use super::assert_typed_execution_fixture;

#[tokio::test]
async fn typed_execution_async_stream_cancel() {
    assert_typed_execution_fixture().await;
}
