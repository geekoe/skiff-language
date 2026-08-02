# Router Rust Migration C-routing-query：exact candidate projection 冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；供 W-routing-query / W-dispatch /
W-activation 共同消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md` §3.2
  （`RuntimeRegistrationDirectory` 双索引、exact registered tuple、
  registration revision；`RuntimeCandidateQuery` 无资格缓存）、§3.3
  （active routing 单一 authority；capture 一次 `Arc<RoutingEpoch>` →
  candidate query → `RegisteredSessionLease`；heartbeat 不参与 admission；
  old request 持旧 epoch 延续，无全局 pin map）、§3.4（identity/fence）、
  §5.4（C-routing-query → W-routing-query；W-dispatch/W-activation 共同
  消费）。冲突时以权威设计为准。
- 父批次：`doc/implementation/router-rust-migration-batch-4.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration-contracts-request-leaf.md`。
- 同链契约：`router-rust-migration-c-bootstrap-contract.md`
  （`RoutingEpoch` 字段语义与 atomic publication）、
  `router-rust-migration-c-session-contract.md`
  （`RegisteredAssemblyTuple`、revision、cancellation、replacement）、
  `router-rust-migration-c-dispatch-contract.md`（admission 流水线消费本
  pack 输出）。

## 1. 冻结范围

冻结 **exact candidate projection**：给定一次 captured `RoutingEpoch` 与
`RuntimeRegistrationDirectory` 的 exact registered tuple 视图，投影出
`RegisteredSessionLease` 候选集合（§3.3 第 2-3 步）。本 pack 不定义
selection policy / permit / revalidate / enqueue（C-dispatch）；不定义
directory 写路径（C-session）；不写 production。

## 2. 冻结输入与输出

### 2.1 输入

```text
captured_epoch: Arc<RoutingEpoch>
  RoutingEpoch { environment, assembly_generation, assembly_identity,
                 config_snapshot_id,
                 immutable ingress/deployment/actor routing projection }
directory_view: 每 session 的 exact facts：
  SessionRecord { session_epoch: RuntimeSessionEpoch,
                  registered_tuple: Option<RegisteredAssemblyTuple>,
                  registration_revision: u64,
                  cancelled: bool,
                  capabilities: DispatchCapabilities }
  DispatchCapabilities { unary: bool, server_stream: bool }
query: { mode: unary | serverStream,
         deployment: ServiceDeploymentRef（来自 captured epoch 的 exact
                     deployment 投影） }
```

`RegisteredAssemblyTuple { environment, generation, assembly, config_snapshot }`
与 `RuntimeSessionEpoch { replica_id, connection_generation }` 的语义沿用
C-session 契约；`RoutingEpoch` 字段语义沿用 C-bootstrap 契约。

### 2.2 输出

```text
Vec<RegisteredSessionLease>
RegisteredSessionLease {
  session_epoch: RuntimeSessionEpoch,
  registration_revision: u64,
  exact_registered_tuple: RegisteredAssemblyTuple,
  cancellation: CancellationToken,   // 借 directory 语义；candidate 不持有真 token
  capabilities: DispatchCapabilities,
}
```

查询返回全部 exact 候选（无排序承诺；排序/选择归 `RuntimeAdmissionPool`）。

## 3. 投影规则（冻结）

1. **whole-epoch capture**：查询只使用调用方一次捕获的 epoch 引用；禁止
   混合不同 epoch 的 tuple/字段（§3.3）。
2. **exact tuple 匹配**：候选 session 的 `registered_tuple` 必须与 captured
   epoch 的 `(environment, assembly_generation, assembly_identity,
   config_snapshot_id)` 逐字段相等。deployment 坐标匹配由调用方 query 的
   exact deployment 决定（`serviceId`/`contractVersion`/
   `deploymentRevision`/`deploymentArtifactIdentity` 全部相等）；本 pack
   冻结：candidate 的 tuple 匹配是 epoch 级匹配，deployment 匹配是
   query 级匹配。
3. **一个完整 revision**：directory_view 中每个 session 只暴露一个完整
   revision（tuple + revision 同一快照）；投影读取该 revision，不拼接。
   revision 非当前值（如 C-session replacement 竞态中的旧 revision）视为
   stale，不作为候选。
4. **cancelled 排除**：`cancelled == true` 的 session 绝不进入候选
   （§3.3：拒绝 cancelled session）。
5. **capability 匹配**：`mode == unary` 要求 `capabilities.unary`；
   `mode == serverStream` 要求 `capabilities.server_stream`；不满足则排除。
6. **heartbeat 不参与**：session 的 health/heartbeat freshness 不进入
   candidate query（§3.3；`RuntimeHealthLedger` 只服务 health projection）。
7. **capability 缺失时 fail closed**：exact tuple 无候选 → 空结果；由
   admission 层映射为 fail closed（C-dispatch）。
8. **多 replica**：多个 replica 注册同一 exact tuple 时全部返回（同一
   session 只返回一次；`current_by_replica` 保证每 replica 一个 current）。

## 4. Corpus 规格

位置：`runtime/transport/testdata/routing-query/scenarios/`。

```json
{
  "schemaVersion": 1,
  "scenario": "<name>",
  "directoryRevision": 1,
  "directoryCurrentEpochGeneration": 43,
  "epoch": {
    "environment": "prod", "generation": 42,
    "assemblyIdentity": "skiff-runtime-assembly-v3:sha256:<64 hex>",
    "configSnapshotId": "skiff-runtime-config-snapshot-v1:<32 hex>",
    "deployment": {
      "serviceId": "example.com/service-1", "contractVersion": "1.0.0",
      "deploymentRevision": "deployment-1",
      "deploymentArtifactIdentity": "skiff-deployment-artifact-v4:sha256:<64 hex>"
    }
  },
  "query": { "mode": "unary | serverStream" },
  "sessions": [
    {
      "id": "s1",
      "sessionEpoch": { "replicaId": "runtime-a", "connectionGeneration": 1 },
      "revision": 1,
      "registered": true,
      "tuple": { "environment": "prod", "generation": 42,
                 "assembly": "skiff-runtime-assembly-v3:sha256:<64 hex>",
                 "configSnapshot": "skiff-runtime-config-snapshot-v1:<32 hex>" },
      "cancelled": false,
      "capabilities": ["unary", "serverStream"],
      "heartbeatFresh": true
    }
  ],
  "expect": {
    "candidates": ["s1"],
    "note": "<语义说明>"
  }
}
```

- `directoryRevision`（默认 1）：directory_view 的当前 revision；session
  `revision` 不等于它时视为 stale，不作为候选。
- `directoryCurrentEpochGeneration`（可选）：directory 当前 epoch 的
  generation，仅用于记录"capture 是 whole-epoch lease"语义；查询只使用
  `epoch`（captured epoch）字段，不读取该字段。
- `heartbeatFresh`（默认 true）：仅文档字段；查询忽略 heartbeat freshness
  （场景 08 冻结该规则）。

必选场景：

- `exact-single-candidate`
- `multiple-replicas-exact`
- `cancelled-excluded`
- `stale-revision-excluded`
- `tuple-assembly-mismatch-excluded`
- `tuple-config-snapshot-mismatch-excluded`
- `capability-server-stream-missing-excluded`
- `heartbeat-freshness-ignored`
- `epoch-capture-is-whole-lease`

消费测试：`runtime/transport/tests/routing_query_corpus.rs`（reference
projection 逐场景断言；场景文件存在性断言）。

## 5. §5.4 contract pack 必填项

### 5.1 owner / invariant

- Owner：stateless `RuntimeCandidateQuery`（显式 `Arc<RoutingEpoch>` +
  directory 只读视图的纯投影；无独立 index、无 refresh、无缓存）。
- Invariant：candidate 集合只由 captured epoch 与 directory 的 exact
  registered tuple/revision/cancellation 决定；cancelled session 永不被
  选择；一次查询只读一个完整 revision；heartbeat/health 永不影响
  candidate 资格；查询无副作用。

### 5.2 typed inputs / outputs

- Inputs：captured `Arc<RoutingEpoch>`、directory_view（typed
  `SessionRecord` 列表）、`Query { mode, deployment }`。
- Outputs：`Vec<RegisteredSessionLease>`（empty = fail closed 信号）。

### 5.3 capacity

- 查询无 mailbox；输入长度受 directory session 总数上限
  （`runtime.maxConcurrency`，C-config）约束；单次查询 O(sessions)。
- 不持有 permit、不占用 admission 容量（permit 归 C-dispatch）。

### 5.4 queue full

- 无队列；directory_view 读取失败 / revision 撕裂（一次视图内出现两个不同
  revision 快照）→ 查询 fail closed（空结果 + 错误），不部分投影。

### 5.5 timeout / disconnect / replacement / shutdown terminal

- 查询本身无 deadline（同步纯投影）；captured epoch 的旧 lease 延续
  （§3.3），replacement/shutdown 不取消已捕获 epoch。
- disconnect/replacement：directory 先 cancel 旧 session 再安装新 current
  （C-session）；本 pack 冻结投影对 cancelled 的直接排除；被替换旧 session
  的 pending 由 C-dispatch 依其 cancellation token 终结。
- shutdown：无进行中查询；进程重启清除全部 ephemeral state。

### 5.6 health fields

- `routingQuery.{queries,candidatesReturned,excludedCancelled,
  excludedStaleRevision,excludedCapability,excludedTupleMismatch}`；
  由 W-routing-query 实现计数；不暴露 payload/tuple 内容。

### 5.7 fake seam

- `FakeRoutingEpoch`（固定 epoch）、`FakeDirectoryView`（固定 SessionRecord
  列表）、`FakeCancellationToken`（已取消/未取消）。corpus 测试使用
  fixtures + reference projection；W-routing-query 必须用同一 fixtures。

### 5.8 real boundary probe（定义）

- `router-live:routing-query`：真实 `RuntimeRegistrationDirectory`
  （W-session 交付）注册多 replica 后，对真实 `RoutingEpoch` 执行 candidate
  query；随后 cancel/replace 一个 session，断言候选集合与 corpus 场景一致、
  heartbeat 更新不改候选。该 probe 在 W-routing-query + W-session 合入后
  成为 `router-rust-routing-query-live` managed probe。

## 6. W-routing-query 交付义务（非本包实现）

1. 实现 `RuntimeCandidateQuery` port 并消费本 corpus 全部场景。
2. 与 C-session 的 `RuntimeRegistrationDirectory` 对接：查询读取一个完整
   revision；replacement 竞态下不产生混合 tuple。
3. 与 C-dispatch 的 `RuntimeAdmissionPool` 对接：返回 typed lease 供
   reserve/revalidate。
4. 不新增独立 eligibility 缓存或 heartbeat freshness 输入。
