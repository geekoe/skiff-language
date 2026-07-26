# P5-F361 package-test gateway entrypoint

状态：Ready（C3 runtime/package-test leaf；与Router snapshot迁移并行）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F358-runtime-assembly-http-gateway-linking-result.md`
- `P5-F359-http-gateway-request-protocol-result.md`
- `P5-F267-inline-effect-runtime-cutover-result.md`

以上父节点沿引用链连接唯一权威设计与测试语义：

- `../../../../architecture/package-service-contract-deployment.md`
- `../../../../reference/testing.md`

本任务只把`skiff-runtime-package-test`从旧ServiceContract operation entrypoint迁到F358的exact linked
gateway entry；不修改test-runner producer、Host request执行、Router或inline effect语义。

## Exact base

- integration commit：`b4a03c26d9a74a1ce026d36f816020069f972535`
- integration tree：`833af5f87aa8a65732c2535fa7844d42f5710bac`
- branch：`codex/package-service-phase-05`

当前`runtime/package-test/src/lib.rs`仍把`PackageTestEntrypoint`定义为
`deployment + contract + ContractOperationId`，并尝试从已经迁移为`LinkedGatewayEntry`的
`candidate.ingress(selector)`读取旧字段，导致该crate及Host依赖编译失败。

## 必须完成

1. `PackageTestEntrypoint`改为精确gateway引用：
   - test-owned id；
   - exact `ServiceDeploymentRef`；
   - exact `GatewayEntryKey`；
   - exact `GatewayEntryIdentity`。
   不再保存`ServiceContractRef`或`ContractOperationId`。
2. Template validation只使用F358 linked candidate：
   - deployment必须存在；
   - `(deployment,key)`必须命中`LinkedGatewayEntry`；
   - entry identity必须逐值相等；
   - entry owner必须等于deployment；
   - test entry必须是HTTP unary；当前test case不允许server stream或WebSocket；
   - duplicate/empty id、missing/wrong deployment/key/identity/mode全部fail closed。
3. `ingress_entrypoint(selector)`通过candidate的selector lookup取得同一个linked entry，再按
   deployment/key/identity精确匹配test-owned entrypoint；不得使用display/source path、contract operation
   或短名fallback。
4. `LoadedPackageTestRuntimeProgram`提供handler的exact `OperationTargetRef`（可把旧
   `operation_target`改成职责准确的名称）；target只能来自`LinkedGatewayEntry.handler`。
5. 迁移`runtime/package-test`直接fixtures/tests：
   - 普通package、package dependency与internal service dependency链仍可装配；
   - ingress现在证明selector和test entry共享同一个gateway entry；
   - handler target可执行地址保持正确；
   - wrong key/identity/mode/owner负例；
   - internal service-call operation模型继续存在，不能误删。

## 写入范围

允许：

- `runtime/package-test/**`；
- 仅为编译该crate所需的局部import/API命名调整。

禁止：

- `test-runner/**`；
- artifact/deployment/RuntimeAssembly/loader/linker共享DTO或identity；
- Host/request/eval/transport；
- Router、compiler、三仓库service、stable/live配置、lockfile。

若linked gateway entry现有API不足且必须改F358公共模型，立即返回`TASK_SCOPE_EXPANDED`。

## 验证

```bash
cargo test -p skiff-runtime-package-test -- --list
cargo test -p skiff-runtime-package-test
cargo check -p skiff-runtime-package-test
rustfmt --edition 2021 --check <changed-rust-files>
git diff --check
```

先确认selector非零。反向搜索production `runtime/package-test/src`不得剩余
`ContractOperationId|ServiceContractRef|contract_operation_id|operation_descriptor`。不运行
workspace/root、stable/live，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f361-package-test-gateway`
- branch：`codex/p5-f361-package-test-gateway`
- 从包含本task的integration checkpoint创建；
- production/tests一个commit，result一个commit；
- result写入`P5-F361-package-test-gateway-entrypoint-result.md`；
- worktree保持clean，不merge/rebase integration，不push。
