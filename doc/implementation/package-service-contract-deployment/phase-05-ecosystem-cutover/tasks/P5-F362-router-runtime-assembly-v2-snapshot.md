# P5-F362 Router RuntimeAssembly v2 snapshot

状态：Ready（C3 Router snapshot/loading leaf；与runtime/package-test迁移并行）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F348-external-ingress-runtime-router-audit-result.md`
- `P5-F358-runtime-assembly-http-gateway-linking-result.md`
- `P5-F359-http-gateway-request-protocol-result.md`

以上父节点沿引用链连接唯一权威设计：

- `../../../../architecture/package-service-contract-deployment.md`
- `../../../../architecture/gateway-runtime-adapter-boundary.md`
- `../../../../reference/runtime.md`

本任务只迁移Router对不可变RuntimeAssembly/ServiceDeployment的HTTP snapshot读取与exact join；不修改
HTTP socket dispatch、runtime registry/request builder、Host、test-runner或WebSocket业务消息模型。

## Exact base

- integration commit：`b4a03c26d9a74a1ce026d36f816020069f972535`
- integration tree：`833af5f87aa8a65732c2535fa7844d42f5710bac`
- branch：`codex/package-service-phase-05`

当前Router仍读取`skiff-runtime-assembly-v1/globalIngress/ContractOperationId`，并加载
ServiceContract推导ingress mode；filesystem loader还在TypeScript中重算Rust-ownedassembly与service
protocol identity。F358/F359后这些都是明确downstream残余。

## 目标snapshot

Router ingress index中的每个HTTP binding至少保留：

```text
selector
exact ServiceDeploymentRef
GatewayEntryKey
GatewayEntryIdentity
adapter kind
dispatch mode
optional deployment policy.timeoutMs
```

它不保存或推导`ContractOperationId`、ServiceContract operation、handler/pre/guard、adapter args、
Package callable或业务schema。Router只用mode选择unary/server-stream，用identity构造F359 routing，用
policy生成deadline。

## 必须完成

1. `runtimeAssemblySnapshot.ts`严格迁到v2：
   - schema/prefix只接受`skiff-runtime-assembly-v2`；
   - required `gatewayIngress`，拒绝`globalIngress`；
   - selector当前只接受HTTP，method required string，host/path继续canonical；
   - binding严格读取`deployment/gatewayEntryKey/gatewayEntryIdentity`；
   - 删除contract operation mode lookup和WebSocket selector分支。
2. Filesystem loader按每个`resolvedDeployments`的canonical record path加载exact
   `ServiceDeployment`：
   - canonical路径使用service/version/revision/deployment identity；
   - strict JSON、bounded file与path containment保持；
   - record的contract坐标、revision、identity必须与ref精确相等；
   - 从deployment `gatewayEntries[key]`读取identity、HTTP protocol surface与policy；
   - assembly binding必须与deployment `ingress selector -> key`及entry identity逐项一致；
   - missing/extra/duplicate selector、missing key、wrong identity、non-HTTP protocol、非法
     typedJson server-stream全部fail closed。
3. Router snapshot只保留dispatch所需的`adapterKind`、`operationMode`和`timeoutMs`；不得把
   handler/pre/guard/adapter plan或external schema带入request builder。
4. 删除filesystem loader内RuntimeAssembly与ServiceProtocol identity的TypeScript hash重算。
   Router仍校验canonical path、declared exact identity、strict lexical与交叉引用；内容identity的唯一
   producer/validator保持Rust artifact owner，不复制算法。
5. 保留`resolvedContracts`引用供internal service deployment/registration consumer使用，但HTTP ingress
   不加载ServiceContract record、不从operation推导mode。
6. 更新直接snapshot/filesystem/activation tests，至少覆盖：
   - raw unary、raw server stream、typed unary；
   - assembly/deployment exact join；
   - optional timeout与无override；
   - v1/globalIngress/operation/WebSocket、wrong key/identity/mode/policy负例；
   - activation snapshot replace与generation规则不回归。

## 写入范围

主要owner：

- `router/src/router/runtimeAssemblySnapshot.ts`；
- `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts`；
- 直接snapshot/activation/filesystem fixture与tests；
- 若类型投影直接需要，`assemblyActivationCoordinator.ts`的薄适配。

禁止：

- `assemblyHttpGateway.ts`、`assemblyRuntimeRegistry.ts`、`runtimeDispatcher.ts`；
- `gateway/assemblyWebSocketGateway.ts`及WebSocket业务/connection模型；
- shared protocol owner、Rust artifact/identity/Runtime；
- test-runner、compiler、三仓库service、stable/live配置、lockfile。

完整Router type-check仍会被尚未迁移的request builders/WebSocket consumer阻断；本leaf必须让owned
snapshot路径及聚焦tests无错误，并在result精确列出剩余consumer。若正确join需要Router复制完整Rust
deployment identity算法，立即返回`TASK_SCOPE_EXPANDED`。

## 验证

先枚举非零test files，再运行：

```bash
pnpm --filter @skiff/router exec vitest run \
  tests/active-assembly-reload.test.ts \
  tests/assembly-runtime-endpoint.test.ts \
  tests/host-ingress.test.ts
pnpm --filter @skiff/router exec tsc --noEmit --pretty false
git diff --check
```

若现有直接filesystem tests在其它文件，使用实际非零集合并记录。完整type-check允许仅由禁止范围内的
明确下游consumer失败；owned文件不能报错。反向搜索owned production路径无
`globalIngress|contractOperationId|skiff-runtime-assembly-v1|computeRuntimeAssemblyIdentity|
computeServiceProtocolIdentity`生产残余。不运行stable/live，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f362-router-assembly-snapshot`
- branch：`codex/p5-f362-router-assembly-snapshot`
- 从包含本task的integration checkpoint创建；
- production/tests一个commit，result一个commit；
- result写入`P5-F362-router-runtime-assembly-v2-snapshot-result.md`；
- worktree保持clean，不merge/rebase integration，不push。
