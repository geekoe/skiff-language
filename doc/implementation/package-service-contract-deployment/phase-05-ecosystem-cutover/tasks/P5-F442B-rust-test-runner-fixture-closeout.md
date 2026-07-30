# P5-F442B Rust / Host / test-runner fixture closeout

状态：Ready。只修current-positive Rust/test-runner fixture，关闭cheap combined的R节点。

## 直接父节点

- `P5-F442A-final-fixture-tooling-preflight-result.md`

父审计已实测：

- package-test两个initializer缺`collection_name_mapping`，目标尚未编译；
- Host lib为304 pass / 4 fail，四项均被current-positive RuntimeAssembly v1遮挡；
- test-runner integration为27 pass / 1 golden fail / 1 ignored；
- test-only `register_mapper`无production consumer，仍自证旧receive/Gateway v1树。

实现基线为 `0303fe5d`。

## 目标与写集

只允许修改：

- `runtime/package-test/tests/support/mod.rs`
- `runtime/host/src/host/router_session/tests.rs`
- `runtime/host/src/eval_capability_adapter/actor.rs`
- `runtime/host/src/eval_capability_adapter/request_contexts.rs`
- `runtime/host/src/capability_context/actor/tests.rs`
- `runtime/host/src/capability_context/outbound_service.rs`
- `runtime/host/tests/active_runtime_assembly.rs`
- `runtime/host/src/host/mod.rs`
- 删除 `runtime/host/src/host/register_mapper.rs`
- `test-runner/src/runtime_execution/tests/orchestration.rs`
- `test-runner/tests/package_service_contract_deployment.rs`
- 本节点result

要求：

1. 两个missing mapping按current type使用语义正确的empty mapping，不修改production结构；
2. Host current-positive identity刷新为RuntimeAssembly v2、ServiceProtocol v5；明确stale negative保留；
3. 删除仅由`#[cfg(test)]`声明的旧register mapper及module声明；
4. test-runner orchestration的DeploymentArtifact positive刷新为v3；
5. WebSocket source fixture必须重新核对compiler生成的完整build/ABI/deployment/assembly tuple；
   不得只把第一个expected hash替换成一次actual后停止。

禁止修改Rust production、compiler/artifact算法、cross-system corpus、checker、README或其它task/result。
若golden变化要求production owner或公共语义变化，停止并返回 `TASK_SCOPE_EXPANDED`。

## Test-first与验证

先重跑父审计的至少一个真实失败作为RED。完成后运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-package-test --test package_artifact \
  entrypoint_validation_rejects_non_exact_gateway_facts
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-host --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-test-runner --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-test-runner --test package_service_contract_deployment
cargo fmt --check
git diff --check
```

记录准确test count。不得启动stable、network、live或完整workspace suite。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f442b-rust-fixtures`
- branch：`codex/p5-f442b-rust-fixtures`
- result：`P5-F442B-rust-test-runner-fixture-closeout-result.md`

Implementation与result分开提交。5分钟内开始实际fixture修改；不得派子Agent，不得
merge/rebase/push。
