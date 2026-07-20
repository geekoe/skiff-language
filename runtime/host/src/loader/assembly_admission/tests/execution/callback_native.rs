use super::assert_typed_execution_fixture;

#[tokio::test]
async fn typed_execution_callback_native() {
    assert_typed_execution_fixture().await;
}
