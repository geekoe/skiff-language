# P5-F445H I7 P8 K1 Generated test service ID simplification

状态：

```text
IMPLEMENTED
READY_FOR_INTEGRATION
```

## 1. Parent, baseline, DAG

- 直接父节点：
  `P5-F445H-I7-P8-K-test-runner-http-entry-closure-result.md`
- 该父节点沿 `P8-D0` 追溯到唯一权威设计和测试语义文档。
- baseline commit：
  `9fd0fc003b8edd0bdb8fdd7626cfa5c7f6b1de22`
- baseline tree：
  `97a524b867b02ebeec2495243ad5f34518556e63`
- DAG：`P8-K -> P8-K1 -> P8-T`
- integration owner：`/root/phase05_integration_steward`

## 2. Goal and preflight facts

把 test-runner 生成的 case service ID 从：

```text
test.skiff/package/<safe-package-id>/case-<index>
```

硬切为：

```text
test.skiff/<safe-package-id>/case-<index>
```

零 worktree 只读预检确认：

- canonical owner 只有 `test-runner/src/package_test_assembly.rs`；
- 直接消费者只有 ecosystem smoke oracle、其 fixture helper 和 I02 combined fixture；
- `test.skiff/ecosystem-smoke`、`test.skiff/package-service-*` 和 runtime orchestration 中的手写
  service ID 不属于该生成格式，不修改；
- 不需要 Router、Runtime、compiler、std、schema 或 artifact 改动。

## 3. Write set and non-goals

预期写集：

```text
test-runner/src/package_test_assembly.rs
test-runner/tests/package_service_contract_deployment.rs
scripts/lib/package-service-ecosystem-smoke-oracle.mjs
scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs
scripts/tests/package-service-i02-combined.test.mjs
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-P8-K1-generated-test-service-id.md
  P5-F445H-I7-P8-K1-generated-test-service-id-result.md
```

不改手写 ID，不增加兼容、dual path、解析 fallback 或新公共表面。

## 4. RED / GREEN and completion

RED：在 baseline 生成两个 case，精确断言新格式；旧生成器必须失败。

GREEN：

```text
cargo test --locked -p skiff-test-runner explicit_test_service_http_entries_are_projected_per_case_without_subject_ingress
node --test scripts/tests/package-service-i02-combined.test.mjs
node --test scripts/tests/package-service-ecosystem-http-fixture.test.mjs
cargo check --locked -p skiff-test-runner --tests
cargo fmt --all -- --check
git diff --check
```

反向搜索当前 production/oracle/test 表面中的旧 canonical pattern 必须为零；历史 result 追溯文本可保留。
完成后提交 implementation/result 并交给 integration owner；不 merge、不 push。
