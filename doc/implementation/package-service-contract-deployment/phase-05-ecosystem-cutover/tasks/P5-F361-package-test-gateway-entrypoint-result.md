# P5-F361 package-test gateway entrypoint result

状态：Completed（C3 `runtime/package-test` consumer；test-runner producer、Host request执行、
Router与inline effect语义未修改）。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| integration base | `b4a03c26d9a74a1ce026d36f816020069f972535` | `833af5f87aa8a65732c2535fa7844d42f5710bac` |
| task checkpoint | `001054561d10f996f3a7287f484576b73eddb9e5` | `911246e79f1220aad620674e4f55a9c407e83fef` |
| production/tests | `bce4a71d2dd4a81bdf3a3ac8b154a423d5c8ca0f` | `f5ce3259d53b6daf0034e32c3b8045388e303b67` |

工作分支为`codex/p5-f361-package-test-gateway`，worktree为
`/Users/geek/workspace/skiff-p5-f361-package-test-gateway`。本leaf没有merge/rebase integration，
没有运行workspace/root、stable/live，没有修改test-runner、Host、Router、shared DTO/identity、
lockfile或三仓库service源码，也没有push。

## 2. Exact package-test entrypoint

- `PackageTestEntrypoint`现在只保存test-owned `id`、exact `ServiceDeploymentRef`、
  exact `GatewayEntryKey`与exact `GatewayEntryIdentity`；production不再导入或保存
  `ServiceContractRef`/`ContractOperationId`。
- template先要求deployment activation存在，再通过
  `AssemblyLinkedCandidate::gateway_entry(deployment, key)`取得F358 linked entry。声明identity与
  linked identity、linked owner与声明deployment均逐值相等；协议面必须是HTTP `Unary`。
- empty/whitespace id、duplicate id、empty entrypoint set、missing/wrong deployment、wrong key、
  wrong identity与server-stream mode均fail closed。F358 loader仍在linked candidate形成前拒绝
  WebSocket selector，package-test没有增加第二条WebSocket或legacy路径。
- `ingress_entrypoint(selector)`只使用`candidate.ingress(selector)`取得linked entry，再以
  deployment/key/identity三元组匹配test-owned entrypoint。跨deployment即使key和protocol identity相同也
  不会匹配；没有display/source path、contract operation、短名或fallback lookup。
- `LoadedPackageTestRuntimeProgram::handler_target`只从exact linked gateway entry的
  `handler().target()`返回`OperationTargetRef`。

## 3. Direct fixture convergence

- package-only、package dependency与internal service dependency fixtures均生成canonical linked
  gateway entry，并从deployment取得exact key/identity构造test entrypoint。
- fixture把public service operation callable与private external gateway callable明确分开：
  operation binding继续指向canonical public callable；gateway entry只指向
  `PackageLocalAbi.implementationSymbols`中的`InternalFunction` callable。两者可在同一assembly中共存。
- direct fixtures同时补齐当前checkpoint已经required的`CallIr.site`、implementation function link、
  Package schema index resolver与`service_call_roots`字段，并删除已退出canonical shape的
  callable closed-throw和boundary error字段；修改全部局限在`runtime/package-test/tests`。
- provider/consumer正例继续解析activation-relative service call与linked internal operation；
  ingress正例用`Arc::ptr_eq`证明selector lookup与test-owned `(deployment,key)` lookup共享同一个
  `LinkedGatewayEntry`。

## 4. Negative and execution evidence

| 证据 | 覆盖 |
| --- | --- |
| `entrypoint_validation_rejects_non_exact_gateway_facts` | empty/duplicate/zero id set、missing/wrong deployment、wrong key、wrong identity |
| `server_stream_gateway_entry_is_not_a_package_test_case_entrypoint` | valid linked raw HTTP server-stream entry仍被package-test unary gate拒绝 |
| `ingress_selector_does_not_match_a_test_entrypoint_owned_by_another_deployment` | 相同key/identity不能绕过exact deployment owner |
| `package_only_artifact_loads_through_the_typed_assembly_path` | exact entrypoint facts与handler executable target |
| `canonical_package_direct_target_is_linked_into_the_shared_execution_image` | package dependency direct target保持正确 |
| `provider_consumer_service_call_stays_activation_relative_and_ingress_is_canonical` | internal service-call operation与gateway selector/entry同时存在 |

## 5. Reverse search

执行：

```text
rg -n \
  'ContractOperationId|ServiceContractRef|contract_operation_id|operation_descriptor' \
  runtime/package-test/src
```

结果为零匹配。测试fixture中保留的`ContractOperationId`、`ServiceContractRef`与
`contract_operation_id`只用于明确证明internal service-call operation模型仍存在，不参与package-test
external entrypoint production lookup。

## 6. Verification

Selector先枚举并确认非零：

| selector | 枚举结果 |
| --- | --- |
| `skiff-runtime-package-test` | 8 tests |

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-package-test -- --list` | PASS；8 tests，非零 |
| `cargo test -p skiff-runtime-package-test` | PASS；8/8 |
| `cargo check -p skiff-runtime-package-test` | PASS；仅dependency既有dead-code warnings |
| `rustfmt --edition 2021 --check runtime/package-test/src/lib.rs runtime/package-test/tests/package_artifact.rs runtime/package-test/tests/support/mod.rs` | PASS |
| `git diff --check` | PASS |

## 7. 自验收矩阵

| 任务条款 | production/test证据 | 结果 |
| --- | --- | --- |
| exact test-owned gateway引用 | `PackageTestEntrypoint`四字段；production旧operation类型反搜零匹配 | PASS |
| linked candidate strict validation | activation、`gateway_entry(deployment,key)`、identity、owner、HTTP unary逐项验证 | PASS |
| selector exact join | `candidate.ingress`后按deployment/key/identity匹配；shared `Arc`正例与wrong-owner负例 | PASS |
| handler exact target | `handler_target`唯一读取`LinkedGatewayEntry.handler().target()` | PASS |
| direct fixture迁移 | package-only、direct dependency、service dependency、wrong fact/mode/owner probes | PASS |
| internal operation保留 | 同一provider/consumer fixture继续取得descriptor、activation operation与service binding | PASS |
| ownership与运行边界 | diff仅`runtime/package-test/**`与本result；未运行或修改禁止域 | PASS |
