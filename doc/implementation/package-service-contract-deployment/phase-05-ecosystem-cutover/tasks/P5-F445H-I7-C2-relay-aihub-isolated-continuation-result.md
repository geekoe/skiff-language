# P5-F445H-I7-C2 Relay and AIHub isolated continuation result

状态：

```text
BLOCKED
C_COMPLETE = NO
C_CONTINUATION_EVIDENCE_RECORDED = YES
C_ISOLATED_ASSERTIONS_EXECUTED = NO
BLOCKING_ISSUES = 1
```

service-scoped ingress已经独立验收，Relay与AIHub也可以在同一assembly声明同一个
`GET /v1/models`。C的source/graph与focused receipts不再被旧全局route collision遮挡。

但Relay与AIHub的真实isolated service runs都在
`runtime.assembly_candidate_started`之后返回HTTP `504`：

```text
AssemblyActivationRejected: assembly activation prepare timed out
```

两个run都没有进入service test assertions，因此不能完成C或替代其isolated matrix。

## 1. Exact inputs

| 项 | 值 |
| --- | --- |
| Skiff acceptance baseline | `54ef44d0ed6a22f495be3509c273d24852521cf1` / `bb1a8f719e5d49db74db02164c5f0d76db209ebb` |
| Internals source base | `54286599be3d297f4f8231091f7f78ad61e2c20b` |
| Internals runtime-assembly-v3 mechanical commit | `a3f46c982b7ff92c2f3041c3791db130f193fb70` |
| Internals integrated identity at ledger time | `fb0030be1175c1cc29c572401bcd921aa9676ee3` / `3b42bd3a84aaf4862b414efdb2c8421fe4392adf` |

Internals integration worktree在记录时clean，并包含上述mechanical v3 commit。该commit只接受当前
RuntimeAssembly v3 fixture identity，不改变本次timeout行为。

## 2. Passing evidence

| Evidence | 结果 |
| --- | --- |
| Relay exact service graph | PASS，exit `0` |
| AIHub exact service graph | PASS，exit `0` |
| Agine exact service graph（下游graph continuity） | PASS，exit `0` |
| Relay + AIHub same `GET /v1/models` assembly | PASS |
| same-Host/same-path trusted-header exact deployment dispatch | PASS `1/1` |
| existing Host/service selection suite | PASS `12/12` |
| combined T0 + service receipts | PASS，`47 passed / 2 generated-only skips` |
| Skiff fixed-profile projection exact Rust receipt | PASS |

这些结果证明：

- service-scoped ingress collision已经关闭；
- exact service graphs可构造；
- current v3 fixture/projection可消费；
- Relay/AIHub的同形入口由exact service/version选择。

它们不证明isolated runtime assertions已经执行。

## 3. Blocking evidence

Relay与AIHub各自的isolated run均：

1. 成功进入current isolated workflow；
2. 发出`runtime.assembly_candidate_started`；
3. 在Router activation prepare等待期间超过默认`20s requestTimeoutMs`；
4. 返回HTTP `504`；
5. 没有进入任何service test assertion。

观测到large assembly candidate约`40s`后才开始后续阶段，超过当前20秒prepare budget。按上层约束，
本次没有修改timeout，也没有通过放宽gate掩盖。

准确分类：

```text
C_COMPLETE = NO
C_ISOLATED_ASSERTIONS_EXECUTED = NO
```

这是shared isolated activation/prepare budget blocker，不是新的Relay/AIHub route collision，也不是
service assertion FAIL。

## 4. Scope and next owner

本ledger只记录结果，没有修改Skiff或Internals production/tests。没有运行stable/live/network、shared
Mongo、OAuth或browser。

恢复C需要一个有权处理isolated activation prepare ownership的节点：

- 先定位20秒budget的权威owner与candidate prepare约40秒的耗时组成；
- 不得由Relay或AIHub service source自行改timeout；
- 修复后在精确Skiff/Internals identities上重跑Relay与AIHub isolated matrices；
- 只有非零assertions执行并GREEN后，才能写`C_COMPLETE = YES`。

Skiff ingress/runtime、Internals service source/T0 tooling、assembly prepare budget、fixture identity或
repo identity变化会使本ledger相应证据失效。
