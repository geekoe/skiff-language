# P5-F371 Bootstrap RuntimeAssembly gateway field correction

状态：Ready（fresh artifact bootstrap机械前置；不等同于S7 test-runner ingress迁移）。

## 直接父节点

- `P5-F358-runtime-assembly-http-gateway-linking-result.md`
- `P5-F361-package-test-gateway-entrypoint-result.md`
- 阻塞consumer：`P5-F368-internals-error-payload-marker-cleanup.md`、
  `P5-F369-registry-error-payload-marker-cleanup.md`

F358已将`RuntimeAssembly.global_ingress`替换为HTTP-only `gateway_ingress`，并明确test-runner fixture是
后续机械consumer。当前
`test-runner/src/bin/package_service_smoke_fixture.rs:298`仍用旧字段构造bootstrap empty assembly，
导致所有fresh canonical std bootstrap在Rust编译阶段失败。

## Exact base与必须完成

- Skiff integration：从包含本task的`codex/package-service-phase-05`创建，并在result记录exact
  commit/tree。

1. bootstrap fixture的空assembly只改为当前`gateway_ingress: Vec::new()`字段。
2. 保持bootstrap assembly没有gateway entry、selector、service operation或伪造identity；不把旧
   `globalIngress`数据转换成兼容路径。
3. 本任务不迁移`package_test_assembly.rs`、`ecosystem_smoke_fixture.rs`、test dispatch wire或WebSocket
   fixture；这些仍属于后续S7 owner。
4. 实际运行一次`skiff-package-service-smoke-fixture --bootstrap-only`到fresh temporary artifact root，
   验证返回canonical std package/pointer receipt；不得只做`cargo check`。

## 写入、验证与交付

允许写入仅为：

- `test-runner/src/bin/package_service_smoke_fixture.rs`；
- 该binary的直接局部测试（只有现有测试不能证明bootstrap时才允许）。

禁止其它test-runner synthesis、artifact/deployment/runtime/Router、service源码、lockfile、stable/live。

```bash
cargo check -p skiff-test-runner --bin skiff-package-service-smoke-fixture
cargo test -p skiff-test-runner --bin skiff-package-service-smoke-fixture -- --list
rg -n 'global_ingress' test-runner/src/bin/package_service_smoke_fixture.rs
git diff --check
```

`rg`必须零匹配，测试枚举需如实记录；即使binary无unit test，真实bootstrap receipt仍是必需动态证据。

- worktree：`/Users/geek/workspace/skiff-p5-f371-bootstrap-gateway-field`
- branch：`codex/p5-f371-bootstrap-gateway-field`
- production/tests一个commit，result一个commit；clean，不merge/rebase/push。
- 启动5分钟内开始修改；若还有其它编译owner阻断bootstrap，返回`TASK_SCOPE_EXPANDED`及下一精确错误。
