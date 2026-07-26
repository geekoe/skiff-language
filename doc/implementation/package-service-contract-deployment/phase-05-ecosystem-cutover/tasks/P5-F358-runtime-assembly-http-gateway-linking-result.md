# P5-F358 RuntimeAssembly HTTP gateway linking result

状态：Completed（C3 shared Rust linking checkpoint；request protocol、Host、Router与
test-runner consumer仍未迁移，因此不表示external request已经可执行）。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| integration base | `33cc73d2f3fc942cad6ca69ebdd3fd95d392fb9e` | `0b8642996048c2175300de03032beadc71347731` |
| task checkpoint | `d077abb22de6d208943bc5106d44f037ace7c5e6` | `b6026abb6cbec1e23b350020e919e0b21cf1bd72` |
| production/tests | `338c74384e074b0cc36795b9fcf8f9076fe4121d` | `dba01c4c1506ac4d75eff77095f57e89c4ae1435` |

工作分支为`codex/p5-f358-runtime-assembly-gateway`，worktree为
`/Users/geek/workspace/skiff-p5-f358-runtime-assembly-gateway`。本leaf没有merge/rebase
integration，没有运行workspace/root、stable/live，没有修改lockfile、Host、request、
transport、Router、test-runner、cross-system wire或三仓库service源码，也没有push。

## 2. RuntimeAssembly v2与identity

- canonical schema升级为`skiff-runtime-assembly-v2`，assembly identity marker升级为
  `skiff-runtime-assembly-identity-v2`，identity prefix升级为
  `skiff-runtime-assembly-v2:sha256`。
- 删除`GlobalIngressBinding`和`globalIngress`。required `gatewayIngress`中的每个
  `GatewayIngressBinding`恰好只包含`selector`、`deployment`、`gatewayEntryKey`和
  `gatewayEntryIdentity`；没有serde alias、default、dual read、conversion helper或legacy
  fallback。
- identity preimage完整包含`gatewayIngress`并沿既有unordered collection canonical规则归一化。
  selector、exact deployment ref、key或entry identity任一变化都会改变identity；只改变binding
  插入顺序不会改变identity。
- v1 schema/prefix、旧`globalIngress`、旧contract/operation fields和缺失
  `gatewayIngress`均严格拒绝。surface validation同时证明selector全局唯一、deployment属于
  `resolvedDeployments`、key/identity可按各自strict type重新解析，并让empty assembly继续要求
  empty gateway ingress。
- canonical empty assembly identity更新为
  `skiff-runtime-assembly-v2:sha256:247fc2b3714bf715dc7918a10618be49493645efbbc0f293fc7b3d2e4d32b50f`。

## 3. Assembly union与loader exact join

- `resolve_runtime_assembly`删除`GatewayIngressNotLinked` checkpoint，按每个resolved deployment
  的canonical `ingress`读取selector/key，再只从对应`gatewayEntries[key]`读取identity。
  `BTreeMap<IngressSelector, GatewayIngressBinding>`形成全局canonical union；同一entry的多个
  selector保留为多个binding，跨deployment selector collision与missing key fail closed，
  zero gateway/zero ingress保持合法。
- deployment artifact validation仍先验证entry identity和完整deployment identity，因此错误
  identity不会被assembly重新计算或猜测。
- loader新增独立`runtime_assembly/gateway_ingress` owner。它证明assembly declaration恰好等于
  所有hydrated deployment ingress的canonical union，再按exact deployment/key/identity加入
  deployment entry和exact implementation package。
- handler、optional pre、optional guard只从implementation package的`callableLinks`和
  `packageLocalAbi.implementationSymbols`闭合。map key、nested callable ID、
  `OperationTargetRef`、`InternalFunction` kind和唯一`PackageCallableSignature`逐项验证；
  public/display symbol、dependency package、ServiceContract operation descriptor均不能补全缺失事实。
- 当前selector只接受HTTP；WebSocket selector继续fail closed。hydrated entry保留canonical
  protocol surface、ordered adapter plan与三个callable的exact ID/target/signature。同一
  owner/key由多个selector引用时共享同一`Arc<HydratedGatewayEntry>`。

## 4. Linked candidate API

- 新增职责单一的`LinkedGatewayCallable`与`LinkedGatewayEntry`。entry直接提供owner/
  activation、key、identity、protocol surface、adapter plan，以及handler/pre/guard的exact
  callable ID、target与signature。
- linker按activation的exact implementation build复核linked image callable target，并建立
  `(ServiceDeploymentRef, GatewayEntryKey) -> Arc<LinkedGatewayEntry>`规范化index及
  `IngressSelector -> Arc<LinkedGatewayEntry>`直接typed lookup。
- `AssemblyLinkedCandidate::gateway_entry(owner, key)`和`ingress(selector)`不需要
  ServiceContract lookup；多个selector指向同一entry时保留`Arc::ptr_eq`共享。
- missing activation/entry/image/callable、identity或target错配、重复selector及linked selector
  集合偏离assembly declaration均fail closed。
- `LinkedContractOperation`、`ServiceContractStore`、operation target tables、service binding和
  activation-relative service call保持原样；同一fixture证明internal operation linkage与新的
  gateway entry可以同时存在。

## 5. 自验收矩阵

| 任务条款 | 代码证据 | 测试证据 |
| --- | --- | --- |
| strict v2 surface/identity | `artifact-model/src/runtime_assembly.rs`、`artifact-identity/src/runtime_assembly*` | required/legacy field拒绝、v1拒绝、四字段mutation、reorder稳定 |
| exact assembly projection | `deployment/src/assembly/resolver.rs::insert_gateway_ingress` | zero、one/multiple selector、multiple deployment canonical map、collision/missing key/wrong identity |
| exact loader union | `runtime/loader/src/runtime_assembly/gateway_ingress.rs` | missing/extra binding、wrong identity、shared entry、WebSocket fail closed |
| exact callable/signature | `hydrate_callable`只读implementation link与implementation symbol | private handler/pre/guard正例；missing/public/dependency、nested ID/target、missing/ambiguous signature负例 |
| linked typed entry | `runtime/linker/src/assembly/gateway.rs`与candidate typed lookup | exact owner/key/identity/surface/plan/callables、selector `Arc`共享 |
| internal operation隔离 | 既有contract store和linked operation owner未改名/删除 | 单一fixture同时取得internal operation与linked gateway |
| operation-free external path | 新binding、loader和linker gateway模块不导入contract operation类型 | production反搜零匹配 |

`runtime/linker/src/assembly_execution/service_error_index.rs`仅补齐同crate既有fixture缺失的
`service_call_roots: Vec::new()`并把RuntimeAssembly literal迁到required `gateway_ingress`；
没有改变service error production owner。

## 6. 验证

Selector先枚举并确认非零：

| selector | 枚举结果 |
| --- | --- |
| `skiff-artifact-model runtime_assembly` | 2 tests |
| `skiff-artifact-identity runtime_assembly` | 3 tests |
| `skiff-deployment assembly` | 18 tests |
| `skiff-runtime-loader runtime_assembly` | 17 tests |
| `skiff-runtime-linker assembly` | 25 tests |

| 命令 | 结果 |
| --- | --- |
| 五条`cargo test ... -- --list` | PASS；均非零，见上表 |
| `cargo test -p skiff-artifact-model runtime_assembly` | PASS；2 selected |
| `cargo test -p skiff-artifact-identity runtime_assembly` | PASS；3 selected |
| `cargo test -p skiff-deployment assembly` | PASS；18 selected |
| `cargo test -p skiff-runtime-loader runtime_assembly` | PASS；17 selected |
| `cargo test -p skiff-runtime-linker assembly` | PASS；25 selected |
| `cargo check -p skiff-artifact-model -p skiff-artifact-identity -p skiff-deployment -p skiff-runtime-loader -p skiff-runtime-linker` | PASS |
| changed Rust `rustfmt --edition 2021 --check` | PASS |
| `git diff --check` | PASS |

Production反搜：

```text
rg 'ContractOperationId|ServiceContractRef|operation_descriptor|contract_operation_id' \
  runtime/loader/src/runtime_assembly/gateway_ingress.rs \
  runtime/linker/src/assembly/gateway.rs

sed -n '/pub struct GatewayIngressBinding {/,/^}/p' \
  artifact-model/src/runtime_assembly.rs |
  rg 'ContractOperationId|ServiceContractRef|operation_descriptor|contract_operation_id'
```

两条均为零匹配。owned production paths中的
`GatewayIngressNotLinked|GlobalIngressBinding|global_ingress`反搜同样为零；其它loader/linker
模块中保留的`ContractOperationId`只属于internal service-call graph。

## 7. 已解除但尚未迁移的downstream点

1. `runtime/host/src/loader/assembly_admission.rs`仍使用`GlobalIngressBinding`、
   `assembly.global_ingress`和旧candidate ingress shape；后续Host consumer必须改为exact-match
   selector、linked entry identity与activation。
2. `runtime/eval/**`、`runtime/linked-program/src/shared_image/tests.rs`、
   `runtime/linked-type-plan/src/assembly_seam.rs`及Host tests中仍有`global_ingress`构造器；
   它们是required v2 field的明确Rust fixture/consumer迁移点。
3. `router/src/router/runtimeAssemblySnapshot.ts`、
   `filesystemRuntimeAssemblySnapshotLoader.ts`和`assemblyActivationCoordinator.ts`仍读取
   v1 `globalIngress`及旧identity projection；Router snapshot/dispatch由后续leaf迁移。
4. Rust/TypeScript request routing wire仍携带`contractOperationId`，包括
   `router/src/protocol/runtimeAssemblyRequest.ts`及Host request/admission路径；本leaf没有提前
   定义gateway request codec。
5. `test-runner/src/**`仍含v1 RuntimeAssembly fixtures，`scripts/check-artifact-identity-single-source.mjs`
   与ecosystem smoke oracle仍校验v1 projection/prefix；这些属于后续test-runner、identity
   checker和cross-system corpus迁移。
6. transport/eval/Host/Router中的v1 assembly identity fixtures也仍需随各自consumer迁移。本leaf
   没有用跨owner改动制造workspace绿色。

typed JSON server stream、Router/Host timeout责任、stream framing和WebSocket business entry均未在
本checkpoint决定；它们不影响已经闭合的HTTP linked facts，继续由H36后续consumer leaf拥有。
