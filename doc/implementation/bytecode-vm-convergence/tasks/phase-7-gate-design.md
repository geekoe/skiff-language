# Phase 7 Gate design（read-only preflight 设计稿）

> Status: read-only preparation draft; not an execution baseline or PASS input
>
> Authority: [`phase-7-whole-system-closure.md`](../phases/phase-7-whole-system-closure.md) §4/§5/§6 与
> [`phase-7-execution-map.md`](./phase-7-execution-map.md) §4/§5/§6。本文件只做静态设计，不冻结任何 spec id、
> fixture 或 expectedTests 数字；激活 amendment 从 accepted Phase 6 closeout 动态落账。
>
> 关联 preflight：[`phase-7-activation-preflight.md`](./phase-7-activation-preflight.md)

## 1. 目标与边界

Phase 7 是 closure-only：不改 production 语义，只证明"已 accepted 的 bytecode-only 支持面 + 可执行边界 +
whole-system 组合"。Gate 的证据权威是 C01–C18 矩阵 + 一条真实链路：

```text
client HTTP -> Router gateway/dispatcher -> runtime WebSocket session -> RuntimeHost
  -> atomic image / flat scheduler / VM -> service/task/Actor/interface/callback/DB/recoverable consumer
  -> response / terminal
```

禁用：fake dispatcher frame、hand-built artifact/image/fiber/owner token、test-side 投影语义结果。允许注入
deterministic clock / TaskStore / DB fake / completion race，但只能从 production composition seam 注入。

## 2. 覆盖矩阵（C01–C18）映射到 lane / 生产入口 / 证据

每行由 activation amendment 映射到 exact candidate spec id；accepted capability 必须有正 row，
disabled capability 必须有 fail-closed row，二者不能互换。

| ID | 面 | 主要 owner | 生产入口 | Required expectation | 机器证据 |
| --- | --- | --- | --- | --- | --- |
| C01 | inherited Phase 1–6 closure | P7G | `phase6WorkloadSpecs(root)`（唯一 cumulative 导入） | 每个唯一继承 spec 恰好执行一次；无 nested Gate / 旧 receipt 顶替 / zero/skip/stale / 缺 provenance | spec-catalog digest、per-Phase/lane 覆盖报告、receipts + exact counts |
| C02 | compiler/artifact/image identity | P7P | `.skiff` source → compiler publication → RuntimeHost admission | 候选 schema/ISA/artifact/deployment/image identity 一致；缺失/替换/损坏 fail closed；无 verifier/semantic reconstruction | 动态 identity record、S1–S4 继承 receipts、malformed companion、reverse-search receipt |
| C03 | HTTP unary | P7P | HTTP client → Router → Runtime session → VM → response | exact service/version route；确定性 status/headers/body；单 terminal；owner 平衡 | raw client response、route identity、terminal + inventory receipt |
| C04 | HTTP server-stream | P7P | HTTP client → Router WS→HTTP writer → host stream → provider | headers 先于有序 bounded chunks 与一个 end；cancel/disconnect/error 释放 handle/buffer/pending owner | chunk timeline、backpressure/cancel companion、resource/buffer/pending 归零 receipt |
| C05 | service child | P7P | compiled caller → exact provider build → flat child trampoline → caller response | ledger-selected：accepted = distinct owner/heap success + ordinary throw + actual Pending；disabled = 唯一边界拒绝 | P6 service receipts + whole-system response + owner/root chain |
| C06 | function task + Actor task | P7P | `task-function` / `task-Actor` ingress → host/TaskStore（Actor task 走 exact activation/lease seam） | 两 capability 独立 ledger-selected；accepted 路径保 exact target/build、recoverable payload/materialization、lease/fence、late/duplicate/retry 单 terminal；restart 子集依赖 accepted recoverable；disabled fail closed；TaskStore 与 DB commit 不宣称原子 | per-capability task receipts、build/payload/lease/fence identity、conditional restart/terminal、disabled negatives |
| C07 | interface dispatch | P7P | compiled Local/Remote interface call → exact table/target/carrier → result/error | ledger-selected；accepted 变体只跑 ledger 声明的 variant，exact method/materialization facts；其余拒绝 | dispatch identity、return/error receipt、disabled-carrier negative |
| C08 | callback | P7P | provider callback request → same-Runtime callback owner → caller resume/cancel | ledger-selected；accepted callback 保 lifetime/owner 与 terminal once；cross-Runtime 默认 disabled | callback owner/resume receipt、cancel/late negative、disabled-route evidence |
| C09 | Actor | P7P | exact Actor id/build → Router/runtime arena + lease/fence → result/Pending/destroy | ledger-selected；accepted 覆盖 exact-build 共存、lease/fence/session ownership、Pending、stale/late/destroy；否则 fail closed | Actor route/build/lease/fence receipts、arena/root/resource terminal inventory |
| C10 | DB + recoverable | P7P | DB transaction 与 recoverable codec 边界（独立 ledger-selected） | 每状态 ledger-selected；accepted = schema/materialization/transaction 或 codec cleanup；disabled = 只有 compiler/admission/runtime rejection | DB/codec identity、transaction/roundtrip receipt 或 per-surface disabled negative |
| C11 | cancel/deadline/error mapping | P7P | throw/VmFailure、deadline、client/session disconnect、losing completion | 一个 winner 一个 external mapping；late/duplicate loser 不得发布；unwind/partial/pending/resources exactly-once cleanup | outcome identity、HTTP/error result、race timeline、terminal inventory |
| C12 | lifecycle/resource inventory | P7P | aggregate/exception/Pending/HTTP/stream/cross-owner terminal | copy/move/drop/unwind/materialization 平衡所有 owner/root/resource/buffer/heap counter；无 double release/orphan | before/after inventory、owner cleanup sequence、failure receipts |
| C13 | fuel/frame bound | P7G | request-owned execution budget → VM dispatch/local-call loop | limit N 允许 N 次尝试拒绝 N+1；overflow/deep-call bounded；terminal/settlement exact | 继承 raw-fuel/deep-call receipts + exact counts |
| C14 | memory + conditional GC | P7P | accepted per-request memory ledger 在 aggregate/Pending/stream/cross-owner 压力下 | hard limit 覆盖 heaps/frames/sidecars/pending owners/resources/host buffers/child+Actor owners；失败平衡所有 owner；GC accepted 才在合法 quiescence 压缩，否则无 GC 路由 | ledger charge/limit/peak/terminal receipt、pressure companion、accepted-GC root receipt 或 disabled/deferred proof |
| C15 | deterministic hot-path bounds | P7G | dispatch/cleanup/unwind/wake-claim/stream-pump/materialization-root-walk workload | 每个 accepted loop/queue/buffer 有继承的有限 counter/limit；超界终止或拒绝且无泄漏 | owner workload receipts、limit inventory；wall-clock 仅参考 |
| C16 | capability/observation ledger | P7G | candidate handoff + manifest observations | 12 个 exact capability key 各一个声明状态，与 C03–C10 一致；ordinary = accepted/disabled；`request-GC`/`Actor-compaction` 另保留 disabled/deferred disposition；无 enabled-but-unaccepted | ledger/schema digest、exact-key row reconciliation、unexpected/missing/state-drift negative |
| C17 | hard-cut / damaged-artifact closure | P7G | workspace/production graph + 真实 admission boundary | verifier crate/API/seal/selector/alias/dual path 为零；damaged artifact 只产生 typed construction/safe request failure | reverse search、dependency/selector graph、behavioral damaged-artifact receipts |
| C18 | Gate/evidence controls | P7G | Phase 7 runner/checker self-test fixture | early ordinary red 不截断后续命令；missing/unexpected/zero/skip/stale/tampered/reordered/cross-epoch、active-lease contention、unsafe stale-lease recovery、path escape/symlink/directory swap 各自 FAIL | runner sequence receipts、lease/evidence-root safety probes、independent checker negatives |

## 3. Fixture 复用与新增计划

> 本段是静态设计；activation 时以 accepted Phase 6 fixture 为准核对，不在此处冻结路径。

### 3.1 直接复用 Phase 6 的 fixture 组（S1–S6 继承 sentinel，不改语义）

- service：`service-positive` / `service-provider` / `service-throw` / `service-pending` / `service-negative`
- interface：`interface-local-*`、`remote-interface-*`（含 stream/throw/provider 组）
- callback：`callback-positive` / `callback-pending` / `callback-stream` / `callback-negative` / `containment-cross-runtime-callback`
- recoverable：`recoverable-positive` / `recoverable-restore` / `recoverable-negative` / `containment-cross-service-envelope`
- DB：`db-commit` / `db-abort` / `db-pending` / `db-positive` / `db-negative`
- task：`task-positive` / `task-negative` / `task-actor-method-positive`
- Actor：`actor-positive` / `actor-pending` / `actor-db-only` / `actor-negative`
- containment：`containment-*`（concurrent/serial/gc-compaction/verifier-api/cross-service-envelope）

这些 fixture 在 Phase 6 已流经 S1–S6；Phase 7 通过 `phase6WorkloadSpecs(root)` 原样再执行（re-ID once，保留
provenance），Phase 7 不新建 duplicate fixture。

### 3.2 Phase 7 需新增的 whole-system 组合 fixture（不造 image，只组合 production 链）

| 新增 fixture 组 | 组合内容 | 覆盖 row | 说明 |
| --- | --- | --- | --- |
| `http-unary-*` | 真实 HTTP client → Router gateway → Runtime session → service | C03 | 端到端响应，非 host harness 内投影 |
| `http-server-stream-*` | HTTP stream consumer → Router WS→HTTP writer → host stream → provider | C04 | headers/chunk/end/cancel/disconnect 时序 |
| `cross-capability-*` | 一个请求内 service child + interface + callback + DB + recoverable 组合 | C05/C07/C08/C10/C12 | 组合后 owner/root/resource/buffer 归零 |
| `task-fresh-*` | `task-function` + `task-Actor` 经 TaskStore 的 fresh request 全链 | C06/C09 | restart 子集依赖 accepted recoverable，否则禁用 |
| `memory-pressure-*` | aggregate/child-heap/Pending/stream/cross-owner 压力 | C14 | peak/release 观察 |
| `bounded-work-*` | dispatch/cleanup/unwind/wake-claim/pump/materialization 计数 | C15 | 只引用继承 spec id，不新造 bound |
| `gate-control-*`（纯 P7G，非 fixture） | runner/checker 自测 fixture | C18 | 见 §5 expected-red |

### 3.3 新增 fixture 的 S1–S6 要求

Phase 7 的 whole-system fixture 仍必须满足 sentinel 递进：source→admission→emission→atomic image→scheduler/VM→
request/host/router terminal。同一 fixture identity 从 S1 流到 S6；每个阶段边界一个独立 test case；atomic-link
input 与 image 是同一 constructor 的输入/完成态观察，不建第二个 API。`phase7ScenarioSpecs(root)` 只提供
组合场景，不复制 Phase 6 的 S1–S6 于新名字下。

## 4. 静态 gate-map 预调查：whole-system 场景经过的门与 owner

每条 whole-system 链路在 production pipeline 上经过的门（以 C03 HTTP unary 为例，其余类推）：

| 门 | 位置 | 所有者 | C03 上的验证点 |
| --- | --- | --- | --- |
| 1. source analysis / effects | compiler/source | F1–F6（compiler 唯一 authority） | `.skiff` 的 service/target/effect facts 编译 |
| 2. emission / admission | compiler/emission + artifact-model admission | F6 | 精确 target/build/plan 进 artifact；missing fail closed |
| 3. atomic image construction | linker `DeploymentExecutionImage` 唯一 constructor | F6 + linker | exact resolve，原子发布完整 image |
| 4. admission / activation | RuntimeHost loader / `__skiff/activate-assembly` | X6 + host | image 进入请求入口，identity 一致 |
| 5. scheduler / VM dispatch | flat scheduler + VM | K6 | owner bundle / heap domain / budget |
| 6. request/host/router terminal | host request entry → Router → HTTP writer | X6 + P5 | response terminal；owner/root/resource 归零 |

对每个 row（C03–C15），gate-design 矩阵记录 production entry 与经过门；P7P 在这些门上挂 executable
assertion，P7G 只做 catalog/receipt/checker。disabled capability 的唯一拒绝点（compiler/admission/dispatch）必须
与 Phase 6 ledger 的 fail-closed row 一致，不能靠 message substring 区分。

## 5. Expected-red 计划（closure-only，不破坏 production baseline）

Phase 7 默认不加 production producer，因此不为了 expected-red 故意破坏合法 baseline。P7G 用 self-contained
fake-capture runner fixture 制造受控红（对应 C18 与 MAP7 §6）：

1. early controlled command failure → 后续独立 PASS 命令仍执行、final fresh-status probe 仍成功；
2. dependent command 因 failed producer 变成 `BLOCKED`，证明不消费 stale shared-target binary；
3. missing / unexpected / zero-test / skip / todo / ignored / stale-candidate / reordered receipt / stream-tamper /
   receipt-chain-tamper / env drift / active-lease contention / unsafe stale-lease removal / path escape /
   symlink/directory-swap / cross-epoch：各自独立 FAIL；
4. all-green control 证明 checker 本身能 PASS。

只有 P7O 或原 owner 真的加了 producer 时，对应真实 row 在 join 前保留 nonzero/non-skip expected-red，同时
unaffected 独立 row 继续并出 receipt。

## 6. 能力状态 → 正/负预期选择（ledger-selected）

activation amendment 从 accepted Phase 6 capability ledger 读取 12 个 key 的状态，为每行选择：

- `accepted` → 该 row 用 positive whole-system fixture；
- `disabled` → 该 row 用 fail-closed fixture（唯一拒绝点证据）；
- `request-GC` / `Actor-compaction` → 除 accepted/disabled 外保留 explicit disabled/deferred disposition，并且
  disabled 时不启用 GC 路由，只有"不可达"证明。

任何 key 状态缺失/未识别即 FAIL；generic live smoke 不能升级 ledger。

## 7. P7G/P7P 接口边界（只读阶段已确认）

- `phase7WorkloadSpecs(root)` = `phase7ScenarioSpecs(root)` + 恰好一个 `phase6WorkloadSpecs(root)` 导入（re-ID once，
  保留 `phase6WorkloadProvenance(root)`，不解析嵌套 id 前缀）。
- 动态 identity（schema/ISA/artifact/image/binary/observation/ledger）一律从 candidate production path 读取，不 pin
  literal。
- `bytecode-vm-phase-7-gate` 是 public leaf selector，不在默认 `verify` 展开；`--jobs 1` 串行。
- Cargo：单 epoch `/tmp/skiff-bcvm-p7-r1-cargo.lockdir` + `CARGO_TARGET_DIR=/Users/geek/workspace/.skiff-cargo-target`；
  每个继承 `cargo test` 幂等 `--no-fail-fast`；build/fmt/clippy 不带；不 `cargo clean`。

## 8. Open items / 激活时才可落账

| 项 | 依赖 |
| --- | --- |
| 每个 C-row → exact spec id 映射 | accepted `phase6WorkloadSpecs(root)` + P7P/P7G 资产 |
| 12-key capability ledger 状态 | Phase 6 accepted result |
| `expectedTests` residual inventory（71 missing / 0 null / 24 integer，95 个继承 spec） | P7G adapter catalog 正式落账 |
| G-1 capability ledger export、G-2 observations/memory handoff、G-3 provenance digest | Phase 6 接口缺口（见 preflight §3） |
| evidence epoch `P7-E0` | accepted closeout baseline |