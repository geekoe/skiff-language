# P5-F355 Deployment HTTP gateway model result

状态：Completed（C1 deployment checkpoint；C2 authoring resolution与C3 execution仍显式
fail closed）。

## 1. Exact checkpoint

| 项目 | commit | tree |
| --- | --- | --- |
| 本leaf base / integration checkpoint | `25344ac52c1b87cbec3a3beef9cbf271685e66b6` | `000815606cd47e69e9641c25e9259d1a05634a8e` |
| production/tests | `f9f7a4877a8caba21ec1aa263e4281d8a5912179` | `352cfab0bcf79188c3caa483b34e5cde1be3b3b5` |

工作分支为`codex/p5-f355-deployment-gateway`，worktree为
`/Users/geek/workspace/skiff-p5-f355-deployment-gateway`。本leaf没有merge/rebase
integration，没有运行workspace/root、stable/live，没有push。

## 2. Canonical deployment gateway model

- Shared artifact model新增严格的`GatewayAdapterPlan`与`DeploymentGatewayEntry`。
  `ServiceDeploymentInput`和`ServiceDeployment`共同持有required
  `BTreeMap<GatewayEntryKey, DeploymentGatewayEntry>`。
- Resolved entry精确保存gateway identity、canonical protocol surface、
  handler/pre/guard exact callable ID，以及ordered adapter args。所有新DTO递归
  `deny_unknown_fields`。
- `DeploymentIngressBinding`现在只允许
  `{ selector, gatewayEntryKey }`；旧`contractOperationId`字段和
  `DeploymentIngressBinding -> GlobalIngressBinding`转换已删除。
- `gatewayEntries`使用专用serde map visitor，raw JSON中的duplicate key在反序列化时直接拒绝，
  不经过`BTreeMap` last-write-wins。
- `ServiceDeploymentInput`和`ServiceDeployment`schema升级为v2；deployment identity marker与
  prefix升级为v2。Router snapshot与repo内脚本/fixture仅做对应generation的机械更新，production
  反搜只剩专门验证旧generation拒绝的负例。

## 3. Validation、projection与identity

Input与canonical artifact共用的strict validation现在保证：

- entry identity由F351 canonical protocol surface owner重算且必须精确相等；
- adapter plan kind与HTTP surface kind相等；adapter args直接复用F351 validator；
- `http.context`要求同entry存在pre；
- adapter args中真正跨external boundary的source经wire name规范化、去重后，与surface中的
  canonical external sources精确相等；
- handler/pre/guard callable ID非空；
- 当前generation只接受HTTP selector；
- selector全局唯一、key必须存在、entry不得orphan，同一key允许多个selector；
- zero gateway/zero ingress合法；`operationBindings`本身可为空。

Deployment projection仍先要求operation bindings与传入`ServiceContract.operations`精确闭合，
然后逐值保留validated gateway map和selector/key binding。新增真实zero-operation contract
投影测试，证明只有零operation contract能生成零operation binding deployment。

Deployment identity v2 preimage包含完整gateway map及selector/key binding。`BTreeMap`提供key
canonical order，ingress在identity projection中按selector规范排序；ordered adapter args不排序。
测试逐项证明key、entry identity/surface、handler/pre/guard、adapter param/source和selector
mutation都会改变identity，而map插入顺序、ingress顺序及其它既有unordered binding顺序不会改变
identity；diagnostic text仍不进入preimage。

## 4. Direct consumer checkpoint

| consumer | checkpoint行为 |
| --- | --- |
| compiler generated deployment | 无external authoring时写empty `gatewayEntries`与empty ingress；HTTP和WebSocket authoring都在旧operation resolution前返回明确错误 |
| deployment assembly | nonempty deployment gateway ingress返回`GatewayIngressNotLinked`，不生成旧`GlobalIngressBinding` |
| runtime loader | legacy nonempty `RuntimeAssembly.globalIngress`及任意nonempty deployment gateway surface均明确拒绝 |
| runtime linker | 同样拒绝legacy global ingress和未链接的deployment gateway surface，不产生dispatch map |
| package-test / ecosystem smoke test-runner | 需要external ingress的入口返回not-yet-migrated错误，不把entry重新解释为service operation |
| runtime/Host/package-test fixtures | 只机械补required gateway map和selector/key fixture；没有新增runtime gateway执行 |
| Router/scripts | 只机械切换deployment identity v2 prefix；没有新增gateway DTO、codec或dispatch行为 |

`compiler/driver/generated_deployment.rs`中的`WebSocketRouteAuthoring`、
`resolve_websocket_route`与旧`ingress_bindings`已删除。旧
`compiler/tests/websocket_ingress.rs`正向测试目标也被删除：它既直接构造旧
operation-based deployment ingress，又依赖F354已经禁止的WebSocket public generic authoring。
HTTP/WebSocket fail-closed由generated deployment的lib与真实integration target分别覆盖。

## 5. 自验收矩阵

| 任务条款 | 证据 | 结果 |
| --- | --- | --- |
| typed/raw HTTP | `deployment_gateway_validation_accepts_typed_raw_multiple_selectors_and_zero` | PASS |
| one/one、one/multiple、zero/zero | artifact-identity validation test与projection preservation/zero-operation tests | PASS |
| missing/orphan key、duplicate selector | `deployment_gateway_validation_rejects_cross_field_mismatches` | PASS |
| duplicate raw map key | `strict_wire_rejects_unknown_and_missing_semantic_fields`直接解析含重复`gatewayEntries` key的raw JSON | PASS |
| identity/surface、kind/source、context/pre、callable空值 | artifact-identity cross-field negative matrix | PASS |
| gateway identity mutation矩阵 | deployment identity mutation matrix覆盖key、surface、handler/pre/guard、adapter param/source、selector | PASS |
| reorder稳定 | deployment identity reorder test覆盖gateway map反向插入、ingress反转和既有unordered lists | PASS |
| zero-operation contract/deployment | `projection_accepts_zero_operation_contract_and_empty_gateway_surface` | PASS |
| stale/missing/legacy wire | input/artifact v1、identity v1、missing `gatewayEntries`、旧ingress `contractOperationId`均拒绝 | PASS |
| generated HTTP/WebSocket旧reader | lib 2 tests及integration 10 tests；compiler production反搜无旧resolver | PASS |
| assembly checkpoint | nonempty gateway ingress聚焦测试断言`GatewayIngressNotLinked` | PASS |

## 6. 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-model deployment -- --list` | PASS；1 test，非零 |
| `cargo test -p skiff-artifact-identity deployment -- --list` | PASS；3 tests，非零 |
| `cargo test -p skiff-deployment -- --list` | PASS；52 tests，非零 |
| `cargo test -p skiff-artifact-model deployment` | PASS；1 passed |
| `cargo test -p skiff-artifact-identity deployment` | PASS；3 passed |
| `cargo test -p skiff-deployment` | PASS；52 passed |
| `cargo test -p skiff-compiler --lib generated_service_deployment` | PASS；2 passed |
| `cargo test -p skiff-compiler --test generated_service_deployment` | PASS；10 passed |
| `cargo check -p skiff-artifact-model -p skiff-artifact-identity -p skiff-deployment -p skiff-compiler` | PASS；仅既有warning |
| `cargo check -p skiff-runtime-loader -p skiff-runtime-linker -p skiff-runtime-host -p skiff-runtime-package-test` | PASS；直接production consumer编译通过，仅既有warning |
| `cargo check -p skiff-test-runner` | PASS；仅既有warning |
| changed Rust `rustfmt --edition 2021 --check` | PASS |
| `git diff --check` | PASS |

额外的downstream `cargo check ... --tests`预检会被base已有、与本leaf无关的test fixture漂移阻断：

- `runtime/loader/src/runtime_assembly/tests.rs`、linker fixtures及
  `runtime/linker/src/assembly_execution/service_error_index.rs`仍有旧
  `PackageArtifact` literal缺required `serviceCallRoots`；
- `runtime/package-test/tests/support/mod.rs`还同时引用base已删除的
  `BoundaryErrorContract`/`errors`/`throwTypes`并缺`CallIr.site`。

这些不影响任务指定的聚焦命令或上述production consumer check，没有为制造全仓测试绿色而扩入本leaf。
Router manifest compatibility的额外预检也因worktree未安装router本地依赖而停在
`vitest: command not found`；本leaf没有执行依赖安装。

## 7. 明确残余

1. C2仍负责从strict HTTP authoring解析handler/pre/guard exact callable、generic/source可达性、
   exact linked signature、external schema与完整codec plan。在该接线完成前，任何HTTP authoring
   都明确失败。
2. C3仍负责RuntimeAssembly linked gateway entry、runtime codec、Host/Router dispatch与
   test-runner真实执行。当前Rust assembly/loader/linker与test-runner seam都对nonempty入口
   显式fail closed。
3. Router现有`GlobalIngressBinding -> ContractOperationId`读取与transport operation routing是
   C3 legacy residual；本leaf没有把它改造成gateway执行。新的deployment assembly不会再生成该
   legacy ingress，Rust admission/linker也拒绝其nonempty形态。
4. WebSocket business entry、connect/receive/message DTO与业务消息约定仍未冻结；本leaf只删除旧
   operation-ingress正向路径并明确拒绝现有WebSocket deployment authoring。
5. 未修改F351 gateway identity算法/external schema语义、HTTP authoring shape、transport wire、
   lockfile、本地instance或三仓库service源码；未运行workspace/root、stable/live，未push。
