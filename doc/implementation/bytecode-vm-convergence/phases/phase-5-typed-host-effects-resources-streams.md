# Phase 5：typed host effects, resources and streams

> Status: active; activation authorized by Phase 4 result on main
>
> Semantic Closure: typed registry-to-executor bridge, ResourceTable-owned provider state, real async HTTP, bounded stream/backpressure
>
> Depends on: Phase 4 accepted
>
> Unblocks: Phase 6 cross-owner execution

## 1. 目标

1. 建立 typed registry-to-executor bridge：bytecode host-effect relocation only carries exact binding id; linker typed entry from pinned registry; executor dispatch only by linked target identity, not strings.
2. ResourceTable owns provider state/cancel/drop for the admitted host effects and streams; release/cancel exact once.
3. 真实异步 HTTP（`std.http.client.request`）经 production host boundary returns Ready or Pending without blocking a Tokio worker.
4. bounded stream/backpressure supports two coexisting stream handles with exact routing and bounded buffer; drop/cancel terminates each handle exactly once.
5. Phase 4 pending/root graph remains the single authority; stream/resource roots are inputs only.

## 2. 非目标

Phase 5 不：开启 service/task/interface/callback/Actor；实现 cross-owner heap/request GC；扩大 arbitrary host-effect surface beyond the pinned HTTP and stream handles.

## 3. VCP-5

真实 `.skiff` fixture calls one pinned HTTP host effect and reads a bounded stream; production compiler -> linker -> request path; deterministic in-process host server covers Ready, Pending, timeout/cancel, and two concurrent stream handles. Evidence asserts exact routing, bounded buffer, drop/cancel, and no worker blocking.

## 4. Acceptance checklist

- [ ] typed linked entries from pinned registry only; no string dispatch.
- [ ] ResourceTable-owned provider state and exact cancel/drop.
- [ ] real async HTTP host effect returns Ready|Pending and does not block worker.
- [ ] stream backpressure bounded and two handles route independently.
- [ ] Phase 1/2/3/4 regression + fmt/clippy green.
- [ ] canonical Phase 5 Gate PASS on frozen candidate.

## 5. Amendment r1（2026-08-14）：恢复 epoch 与精确闭包

本 Amendment 是中断恢复后的增量契约；上文仍有效，但与本节冲突时以本节的更窄定义为准。Phase 5 的 exact
baseline 是 Phase 4 已合入 `main` 的 commit
`e643d11fe763200c40b49e24ca922321799278f0`、tree
`c511de9675f6d1a70fd5c995119f44be311cbc4e`；上游 acceptance receipt 是
[`results/phase-4.md`](../results/phase-4.md)。旧 Phase 5 branch、dirty patch、测试输出和 evidence 均未被接受，
不得作为 r1 candidate 的隐式输入。

### 5.1 Exact support ledger

本 Phase 的 host-effect executable identity 闭集只有：

1. Phase 4 回归项 `std.time.sleep`；
2. `std.http.client.request`；
3. `std.http.client.stream`。

`std.http.client.sse`、`core.date.now`、其它 binding，以及仅因同属 `Time` / `HttpClient` context 而匹配的
binding 全部在唯一 admission/link boundary fail closed。context、namespace/symbol、signature、type shape、
string/bytes 或 opcode 存在性都不能替代 exact executable identity。`Stream<T>` 只允许出现在 exact pinned
native result/request-local source slot或 server-stream result authority；operation/gateway 参数、普通 public API
result、用户 record/collection、persistent/global/constant 和无关 function 继续 fail closed。

`std.http.HttpClientStreamHandle` 是含 resource 的 privileged recursive affine composite，不是
`SnapshotShare` 特例。它的 `body` 必须由 verified affine field take 移出；第二次 take/copy/普通 dense-field
share 失败，未取出的 remainder 在 overwrite/return/throw/unwind/request stop 时按 exact recursive plan drop。
source fact → artifact plan → linked plan → verifier placement/take proof → VM physical take/remainder drop 必须闭合；
不得保留按类型名字跳过 embedding 或强制 snapshot 的 bypass。

### 5.2 Typed execution authority

artifact relocation 只携带 exact canonical binding identity。pinned host-effect registry 是 executable identity 的
唯一来源；linker 只有在 exact registry row、signature、authority fingerprint 全部匹配后才 mint 非字符串、闭集
typed executor identity，并把它作为 linked host target 的一部分运输。verifier 证明 relocation → registry row →
linked identity → call/resume facts 的一致性，不重新推导 identity。execution image 只暴露按
`HostEffectAdapterIndex` 对齐的 opaque typed target；scheduler/request exhaustive match 该 identity，禁止读取
binding string、required context、namespace/symbol 或结果外形 dispatch，禁止缺失 provider 时 fallback。

### 5.3 单一 Resource/Pending/root authority

每个 request 只有一个 scheduler-owned `ResourceTable`，由 `RequestExecutionContext` 使用 scheduler-private
resource owner registration 构造；table 是 provider/stream state 的唯一强 owner。它自己 mint 并校验 exact
`(resourceId, generation, owner)`，entry 持 private owner lease、typed pull source/cancel state、single-consumer
lane 和显式 roots；routing 只能持 non-owning reference。unknown、wrong-owner、stale generation fail closed。

release 先在 table 锁内 remove/tombstone，再在锁外执行一次 terminal/cancel/drop 并释放 lease；explicit close
后的 later drop 和 duplicate release 是幂等 no-op。success、ordinary error、cancel、deadline、disconnect 都先
`close_all`，再冻结 Phase 4 owner inventory。ResourceTable/stream roots 只作为 Phase 4 `PendingOwner` root walk 的
输入，不建立第二 registry、root graph、GC authority 或 owner inventory。

ResourceTable 提供 dependency-neutral capability-context `StreamRuntimeApi` 实现并直接持 typed pull source；
production HTTP adapter通过现有 `with_stream_runtime` 注入这一实现。legacy host string-id `StreamRuntime`
registry、adapter singleton、dummy handle 和并行 producer registry 在 bytecode 路径必须保持零 active/不可达。

每个 request 的 sleep、HTTP request/open、`StreamNext`、server-stream emit/backpressure 共用 Phase 4 的同一
`VmPendingRegistry`、wake queue、terminal cell 和 owner inventory。outer request driver 不再把任意 `Parked`
猜成 sleep；sleep timer 只归 typed sleep executor。cancel/deadline/disconnect/host completion 竞争只允许一个
winner，late completion/item 只清理 payload，不能重写 terminal 或 request heap。

### 5.4 Ready/Pending、heap 与 stream

typed executor prepare 出 heap-free future/finalizer，在 scheduler 同步 segment 首 poll 恰一次：

- `Poll::Ready` 直接产生 `Ready(result)`，不 begin/park；Gate 用同一 production HTTP provider 的 deterministic
  pre-I/O validation terminal 覆盖该分支，不把它冒充 network success；
- `Poll::Pending` 先 escrow roots，再向共享 registry begin/publish 并返回真实 `Pending`；Gate 的真实 TCP HTTP
  server 在放行 response/head/chunk 前必须观察到 Parked；
- async task 只持 heap-free typed payload并 settle Phase 4 completion handle；只有 scheduler claim/resume 线程按
  linked result type/shape materialize。不得跨 poll/await 持 heap/resource lock，不得从 Tokio task 读写或锁 VM
  heap，也不得用 nested runtime、blocking join/recv 或额外 OS worker 伪装异步。

两个并存的 HTTP stream handle 必须分别从 table mint exact handle，A 的 next/drop 不能读取或终止 B。VCP 把
buffer capacity 固定为 1，证明 item/end/error 独立 resume、buffer full 时 producer/emit 真实 Pending、消费后
唤醒、wrong/stale ref 拒绝，以及每个 handle normal end/drop/cancel 恰一次 terminal。server-stream response 也
使用同一 request resource/supervisor contract；runtime router-session writer 在送入 WebSocket 前必须有 bounded
mailbox + flush acknowledgement（或等价 production backpressure），不能把 chunk 排入无界队列后宣称已消费。

### 5.5 VCP-5 r1 与 stage sentinels

同一组真实 `.skiff` fixtures 必须从 production compiler publication 连续传到最终 consumer；proof 不得手造
artifact/linked/verified image、executor、resource handle、VM fiber 或 response frame。positive fixture 执行
一次 request effect并打开两个 stream handle，deterministic in-process TCP server/proxy 返回可区分的 A/B
chunks；fixture 经 raw HTTP `serverStream` 把有序结果发给真实外部 client。unsupported companion 至少使用
SSE，并证明 compile/publish fail closed、无 release pointer。

full-chain closure 必须经过：真实 HTTP client socket → Router HTTP gateway → production
`RequestDispatcher`/runtime WebSocket session → `RuntimeHost` admission/request driver → typed outbound HTTP
provider与真实 TCP server → VM stream consumption/server-stream emit → runtime `response.start/chunk/end` 经同一
WebSocket 返回 → Router chunked HTTP response。禁止 fake dispatcher、manual response frames 或只构造 request
ID。Router production 默认 write set 为空；若这条证据定位 Router production 缺陷，立即停止，先 Amendment
MAP 再派 R5-fix owner，proof 不得绕过。

六个独立 stage sentinel 都消费上一阶段的真实产出：

| Sentinel | 必须断言的真实边界 |
| --- | --- |
| S1 source→admission | exact request/stream positive 被接受；SSE/其它 context 负例无 artifact/release pointer |
| S2 admission→emission | 同一 artifact 有 exact adapter/callsite inventory、两次 affine body take、StreamNext 与 recursive drop plans |
| S3 emission→link | production linker从 pinned registry产生 exact typed targets；missing/drift/alias/identity swap 拒绝 |
| S4 link→verify | 同一 linked image 证明 HostEffect/Stream item/end resume、accepted placements、affine take与 remainder drop |
| S5 verify→scheduler | 同一 verified route证明 deterministic Ready terminal、真实 HTTP Parked、共享 registry、两 handle、capacity=1 backpressure与 stale-ref rejection |
| S6 scheduler→request→response | 顶层 `RuntimeHost` 产生 exact ordered response events；terminal/cleanup各一次，pending/resource current=0 且 ever-created=true |

full-chain 另断言真实 outbound target/method 各一次、A/B 不串线、外部 chunk order/body、client disconnect/
timeout/cancel winner、late completion cleanup，以及同一单线程 Tokio worker 在 server gate 未放行时仍可运行 canary，
证明没有 worker blocking。

### 5.6 r1 Acceptance checklist 与停止条件

上文 §4 只有同时满足下列增量条件才算完成：

- [ ] exact baseline、candidate commit/tree 和 evidence epoch 一致，worktree clean；
- [ ] typed identity/affine composite/ResourceTable/Pending/stream/server-stream authority 均只有一个 owner；
- [ ] exact accepted 三 binding 全绿；SSE、date.now、same-context 与非法 Stream placement 全部 fail closed；
- [ ] 六个真实 stage sentinels、full-chain Router VCP、race/negative/structural no-bypass matrix 全绿且非 skip/zero；
- [ ] legacy stream registry active=0，pending/resource terminal 后 current=0，exact-once observations 可核对；
- [ ] Phase 1/2/3/4 canonical regressions与 workspace fmt/clippy/checker 全绿；
- [ ] frozen candidate 经全新 semantic reviewer 与全新只读 Acceptance owner 给出 PASS。

出现下列任一情况停止相应 lane并先 Amend Contract/MAP：需要新增 host binding/Stream placement；需要第二
registry/root/pending/executor authority；需要 hand-built proof seam；需要 Router production 修改；shared typed
identity或affine lifecycle 无法从真实 producer运输到 consumer；或任何 write-set extension。私有、可逆 Rust API
命名留给各 lane，不构成 Contract Amendment。
