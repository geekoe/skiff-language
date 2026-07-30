# P5-F432A Test-runner optional handler checkpoint

状态：Ready。低语义共享compile checkpoint。

## 直接父节点

- `P5-F432-test-runner-unblock-and-combined-wave.md`

父节点记录上游结果、精确候选和被解除的后继；F425A result拥有optional-handler invariant。

## DAG位置

本节点串行解除WebSocket fixture/tooling、AIHub combined和Agine combined。它只适配既定
`DeploymentGatewayEntry.handler: Option<PackageCallableId>`，不迁移旧WebSocket fixture。

## 唯一写入范围

```text
test-runner/src/canonical_test_gateway.rs
test-runner/src/package_test_assembly.rs
test-runner/tests/package_service_contract_deployment.rs
```

以及本leaf result。禁止修改artifact/compiler/runtime/Router schema或execution、其它test-runner
fixture、scripts、Internals和skiff-packages。

## 必须实现

1. `canonical_typed_null_gateway`生成HTTP entry时把exact callable写成`Some(callable_id)`；
   HTTP测试网关不能变成handler-absent。
2. `package_test_gateway_inputs`先fail closed取得handler引用，再与overlay binding callable exact
   比较；错误文本清楚区分missing handler与mismatch，不能格式化`Option`或默认补值。
3. integration test中的两个HTTP handler断言改为exact `Some`/引用比较：
   - package-test unary entry；
   - I02 private HTTP wrapper。
4. 不改变entry identity、adapter plan、selector、zero-operation contract或fixture内容。
5. 反搜所有test-runner `.handler` consumer，证明没有其它仍按必填handler读取的owner；若发现
   production owner超出三个文件，返回`TASK_SCOPE_EXPANDED`。

## 验证

本Agent是以下聚焦证据的唯一owner：

```bash
cargo check -p skiff-test-runner
cargo test -p skiff-test-runner --lib
cargo test -p skiff-test-runner --test package_service_contract_deployment \
  package_test_http_fixture_is_zero_operation_reference_closed_and_fail_closed
cargo test -p skiff-test-runner --test package_service_contract_deployment \
  i02_submit_probe_is_private_http_gateway_not_service_operation
cargo fmt --all -- --check
git diff --check
```

第一个check必须消除已知三错；测试过滤必须实际发现各1项。不得为其它baseline failure扩大范围。
完成后只形成共享实现检查点，full fixture/tooling和service combined由后继唯一owner执行。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f432a-test-runner-handler`
- 分支：`codex/p5-f432a-test-runner-handler`

启动后5分钟内完成第一次实际修改。提交implementation，再新增并提交
`P5-F432A-test-runner-optional-handler-checkpoint-result.md`。返回commit/tree、测试discovery和
clean状态。不得merge、rebase、push、stable/live；完成后不得承接后继节点。
