# P5-F445H-I7-F Test-runner ingress fixture hard cut result

状态：

```text
PASS
F_COMPLETE = YES
HOST_CONTINUATION_UNBLOCKED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```

F已经关闭test-runner与其直接消费的离线source fixture中的旧Host route、旧deployment/assembly identity
和旧runtime frame：package-test control request现在携带精确`ServiceDeploymentRef`，selector只包含
`protocol + method + path`。HTTP URL中的`localhost`只用于形成合法request metadata，不参与路由。

## 1. Parent and exact identities

| 项 | 值 |
| --- | --- |
| direct task | `P5-F445H-I7-F-test-runner-ingress-fixture-hard-cut.md` |
| design parent | `P5-F445H-I7-D0-service-scoped-ingress-design-result.md` |
| canonical parent | `P5-F445H-I7-K-service-scoped-ingress-canonical-result.md` |
| compiler parent | `P5-F445H-I7-C-compiler-ingress-consumer-result.md` |
| baseline commit/tree | `b9aaed250d23f522165136a4cfa35b127d0c8826` / `758fc89311b2f7bbfb8f5d9115eb9aa99d78652d` |
| task commit | `df8473a2` |
| implementation commit/tree | `7b2f5c22a1c6098b08c25cecce55a7ebf93f2180` / `c606a957491e6cb996905f5e4ea814a35daed266` |
| branch | `codex/p5-f445h-i7-f-ingress-fixtures` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-f-ingress-fixtures` |
| integration owner | `/root/phase05_integration_steward` |

最终result commit/tree在Git handoff中报告；result不能自引用自己的commit identity。

## 2. Implementation

- 删除`ecosystem_smoke_fixture`与package-test fixture构造中的`IngressSelector.host`；
- package-test control routing新增精确`deployment`，并保留assembly identity/generation、
  gateway identity和service-local ingress；
- control request URL改为`http://localhost/<path>`，authority只作为HTTP request metadata；
- test-runner response decoder和正向wire fixture升级到`skiff-runtime-frame-v2`；
- 正向fixture/golden升级到ServiceDeployment v4、DeploymentArtifact v4和RuntimeAssembly v3；
- 删除current-scope、encrypted-storage示例service和runtime-live示例service的旧`host: "*"` authoring；
- 精确刷新因authoring和identity hard cut变化的package/deployment/assembly golden。

没有修改compiler、Router、Runtime Host、assembly resolver/loader/linker或运行逻辑。没有运行任何live服务。

## 3. RED and GREEN evidence

### Baseline RED

在精确baseline上运行：

```text
cargo check --locked -p skiff-test-runner
```

真实失败为三处：

```text
test-runner/src/ecosystem_smoke_fixture.rs:84
  IngressSelector has no field named host
test-runner/src/package_test_assembly.rs:255
  IngressSelector has no field named host
test-runner/src/runtime_execution.rs:233
  no field host on type IngressSelector
```

### Final GREEN

| 命令 | 结果 |
| --- | --- |
| `cargo check --locked -p skiff-test-runner --tests` | PASS |
| `cargo test --locked -p skiff-test-runner --no-fail-fast` | PASS，70 passed / 0 failed / 3 ignored |
| `cargo test --locked -p skiff-test-runner runtime_execution::wire --lib` | PASS，10/10 |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

`cargo test --locked -p skiff-runtime-host --no-run`已经成功编译test-runner并越过上述三处，随后停在
`runtime/package-test/src/lib.rs:89`仍以裸`IngressSelector`调用已经改为`ServiceIngressKey`的lookup。
该失败属于并行L consumer production owner，不是F fixture断点，F未越界修改。

## 4. Reverse search

在本任务写集内：

- `selector.host`、`host: "*"`和JSON `"host"`为0；
- 正向runtime frame v1、DeploymentArtifact v3、ServiceDeployment v3和RuntimeAssembly v2为0；
- `test-runner/src/runtime_execution/http.rs`保留的内部`host`字段只保存解析后HTTP authority，用于请求
  `Host` header，不是service route或handler selector；
- previous assembly v1只保留在明确验证旧identity被当前v3拒绝的负例。

## 5. Actual write set

```text
runtime/encrypted-storage-live/default-service/http.yml
runtime/encrypted-storage-live/mapped-service/http.yml
runtime/live-tests/http.yml
test-runner/fixtures/package-service-current-scope/consumer/http.yml
test-runner/src/ecosystem_smoke_fixture.rs
test-runner/src/package_test_assembly.rs
test-runner/src/runtime_execution.rs
test-runner/src/runtime_execution/tests/orchestration.rs
test-runner/src/runtime_execution/tests/support.rs
test-runner/src/runtime_execution/tests/wire.rs
test-runner/src/runtime_execution/wire.rs
test-runner/tests/package_service_contract_deployment.rs
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-F-test-runner-ingress-fixture-hard-cut.md
  P5-F445H-I7-F-test-runner-ingress-fixture-hard-cut-result.md
```

没有push，没有访问stable/network/Mongo/OAuth/browser。

```text
F_COMPLETE = YES
HOST_CONTINUATION_UNBLOCKED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```
