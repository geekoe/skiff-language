# P5-F445H I7 P8 K1 Generated test service ID simplification result

状态：

```text
PASS
TASK_SCOPE_EXPANDED = NO
READY_FOR_INTEGRATION = YES
```

## 1. Baseline and preflight

- baseline commit：
  `9fd0fc003b8edd0bdb8fdd7626cfa5c7f6b1de22`
- baseline tree：
  `97a524b867b02ebeec2495243ad5f34518556e63`
- implementation commit：
  `17aa0cda5751d7c4c377c4c4c3cc67c8c0a1e64d`
- implementation tree：
  `6c8d4b943190458679ff94791b90cd76fa5f3ee7`

零 worktree 只读预检确认 canonical producer 只有
`compile_package_test_contract`。直接 oracle/fixture consumer 只有 ecosystem smoke oracle、
smoke fixture helper 和 I02 combined fixture。现行 `doc/reference` 与 `doc/architecture` 没有旧
canonical 字面。

`test.skiff/ecosystem-smoke`、`test.skiff/package-service-*` 以及 runtime orchestration 中
`test.skiff/package/example` 是不同目的的手写 fixture ID，不属于
`test.skiff/<safe-package-id>/case-<index>` 生成规则，未修改。

## 2. RED and implementation

在两个 HTTP-entry test case 上先加入新格式精确断言，再恢复 baseline 生成器运行：

```text
left:  test.skiff/package/example.com/http-entry-tests/case-0
right: test.skiff/example.com/http-entry-tests/case-0
FAILED
```

GREEN 只删除生成器中的冗余 `package/` 路径段，并同步三个直接 oracle。没有修改 Router、
Runtime、compiler、std、schema、artifact 或手写 service ID，也没有兼容路径。

生成格式现在是：

```text
test.skiff/<safe-package-id>/case-<index>
```

## 3. Evidence

```text
cargo test --locked -p skiff-test-runner \
  explicit_test_service_http_entries_are_projected_per_case_without_subject_ingress -- --exact
PASS (1 passed)

node --test scripts/tests/package-service-i02-combined.test.mjs
PASS (6 passed)

node --test --test-name-pattern='v2 receipt accepts exactly the package-test and probe HTTP gateways|gateway identities, keys, modes and selectors are exact' \
  scripts/tests/package-service-ecosystem-http-fixture.test.mjs
PASS (2 passed)

cargo check --locked -p skiff-test-runner --tests
PASS

cargo fmt --all -- --check
PASS

git diff --check
PASS
```

完整 `package-service-ecosystem-http-fixture.test.mjs` 另有一个与本改动无关的既有失败：
fixture 仍期望已删除的 HTTP `host:` 字段；本任务没有扩大范围修复它。该文件内直接覆盖新
service ID 的两条聚焦测试均通过。

反向搜索：

- production、oracle、test 及现行 reference/architecture 中旧生成格式字面为零；
- 仅保留语义不同的两个 runtime orchestration 手写
  `test.skiff/package/example`；
- Phase 05 历史 task/result 中旧格式作为当时执行事实保留，不作为当前 authority。

## 4. Actual write set

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
