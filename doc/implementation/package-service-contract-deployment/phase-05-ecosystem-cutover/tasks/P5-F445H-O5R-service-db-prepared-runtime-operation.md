# P5-F445H-O5R Service-DB prepared runtime operation

状态：Ready。O5A 之后重发的 service-db provider owner；完成并验收后才能启动 O6 eval DB
状态机。

## 直接父节点

- `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`
- `P5-F445H-O5-service-db-prepared-runtime-operation-result.md`
- `P5-F445H-O5A-prepared-db-capability-seam-result.md`

production prerequisite 为 Skiff integration `c0f68ce0`。O5A 已在
`DbCapabilityStoreApi` / `DbCapabilityStore` 中冻结六个同步 prepare 入口、`Send + 'static`
一次性 wait 和 `FnOnce` finalizer。本节点只实现唯一 production service-db provider，不修改
evaluator transaction/lease 控制流。

## 生产目标

### 1. 完整覆盖 capability seam

`ServiceDbCapabilityStore` 必须逐项覆盖，不能继续使用 O5A 的 fail-closed 默认实现：

- `prepare_find_one_by_key_runtime`
- `prepare_find_one_by_query_runtime`
- `prepare_find_many_page_runtime`
- `prepare_create_runtime`
- `prepare_update_one_runtime`
- `prepare_replace_one_runtime`

调用形状必须保持：

```text
prepare(..., &mut caller_heap, ...)
  -> PreparedDb*RuntimeOperation
  -> owned Send + 'static wait
  -> DbRuntimeFinalizer
  -> finalize(&mut caller_heap)
```

prepare 返回后，wait、wait outcome 和 finalizer 都不得借用或通过 `RuntimeValue` heap handle、
heap index等方式暗中引用 caller heap。跨 wait 保存的数据必须是 provider-owned command、
BSON/document、recoverable context、store/session owner、字符串或其它明确与 caller heap
无关的值。

### 2. Prepare / wait / finalize 的职责

prepare 在当前同步 segment 内完成：

1. type metadata / collection / selector / query / order / projection校验与规范化；
2. 对 write 输入读取 caller heap并编码成 owned BSON/document/update command；
3. recoverable behavior编码、artifact availability校验和 retention root收集；
4. 构造一次性 provider operation；prepare失败不得发起 Mongo command或持久化 retention root。

owned wait 完成：

1. 精确一次取得当前 request DB state；
2. 保持现有 transaction session、lease guard和 lease-lost语义；
3. 在同一 transaction/session中按原顺序持久化 retention roots、执行 Mongo command、file
   cascade和 transaction finish/abort；
4. 返回 raw owned document/value或 provider错误，不接触 caller heap；
5. 首次 poll后被 drop时不得重建/重放 command，也不得把 transaction/session移出原 owner或留下
   无法继续使用的 request state。

finalizer 才重新接收 caller heap：

1. read/update/replace结果按原 metadata、encryption和 recoverable plan物化为 runtime value；
2. find-many保持原顺序并全部成功后返回；
3. create不得把带 caller heap handle的输入跨 wait保存；应从 prepare生成的 owned
   business/storage表示恢复逻辑等价的返回值；
4. decode/coerce/resource失败由 O5A finalizer checkpoint完整回滚本次 allocation；
5. finalizer不重复数据库副作用或 retention root持久化。

### 3. 兼容入口必须只有一套核心实现

既有 heap-borrowing async `*_runtime(..., heap).await` 暂时保留以维持当前调用方编译，但必须是
prepare → 同一个 wait → 同一个 finalizer的薄组合，不能保留第二套 mapping/Mongo状态机，也不能
让新的 prepared入口回退调用旧 async入口。

raw `DbDocument` API、begin/commit/abort transaction、claim/renew/release/read lease和 file
record capability签名不变。本节点不得把 O6 evaluator状态机或 DB语言语义吞进来。

## 必须保持的可观察语义

- ordinary与 recoverable read/write的 BSON、encryption、artifact identity、retention root id、
  expiry和持久化顺序不变；
- transaction内操作继续使用原 session，非 transaction操作继续遵循原自动 transaction条件；
- update/replace的 lease guard、lease-lost标记、immutable-file cascade与错误顺序不变；
- fixed provider/decode/resource错误继续映射成现有 `DbCapabilityError`；
- Ready/Pending都只启动一次 operation；drop/error不重试；
- caller heap在 wait存活期间可独立访问，finalize前 checkpoint、stats、节点和既有内容不变。

若 create返回值无法在不跨 wait保存 caller heap handle的前提下保持语言可观察语义，或 transaction
session无法在 owned wait中保持原 owner/顺序，立即按 `TASK_SCOPE_EXPANDED` 停止并精确上报，
不得用 heap alias、unsafe、全局 registry、重建 future或直接让 eval依赖 service-db规避。

## Test-first 与验收

先写 RED，证明当前 `ServiceDbCapabilityStore` 六个 prepared入口仍落入 O5A 默认拒绝，或 concrete
provider无法形成 heap-free wait。至少覆盖：

- 六个 capability prepare入口全部走 concrete provider，旧 heap-borrowing runtime调用计数为零；
- pending read/write wait存活时 caller heap可继续 mutation，finalize前 caller heap完全不变；
- Ready/Pending只启动一次，unpolled drop、pending drop、provider error均不重启；
- ordinary与 recoverable find-one/find-many/create/update/replace和旧入口结果一致；
- mapping、encryption、retention root identity/顺序、transaction session、lease-lost与file
  cascade不回归；
- create跨 wait不保存 caller heap handle；
- finalize多节点资源失败完整回滚；
- raw DB、transaction和lease API无行为回归。

使用 worktree 专属 target：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5r-service-db/build/cargo-target \
  cargo test -p skiff-runtime-service-db prepared_runtime -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5r-service-db/build/cargo-target \
  cargo test -p skiff-runtime-service-db --locked --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5r-service-db/build/cargo-target \
  cargo check -p skiff-runtime-service-db --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5r-service-db/build/cargo-target \
  cargo check -p skiff-runtime-capability-context --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5r-service-db/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录每项实际测试数；零测试不算证据。不得连接真实 MongoDB、运行 stable/live或访问网络。

## 写集与结构约束

只允许：

- `runtime/service-db/src/capability.rs`
- `runtime/service-db/src/lib.rs`（仅 module声明、薄转发或无法下沉的既有入口）
- `runtime/service-db/src/store.rs`（仅 module声明、薄转发或 request-state owner接线）
- `runtime/service-db/src/mapping.rs`（仅必要的同步 owned encode/decode helper）
- `runtime/service-db/src/prepared_runtime.rs`
- `runtime/service-db/src/prepared_runtime/**`
- `runtime/service-db/src/tests.rs`（仅 module声明）
- `runtime/service-db/src/tests/**`
- 本 result

`lib.rs`、`mapping.rs`、`tests.rs`都已经很长。新增状态机、fixture和行为矩阵必须放窄 child
module；若新文件增长到数百行且同时承担多项职责，应继续按 owner/fixture/test matrix拆分，不能
把 prepared实现重新堆回长 root。

不得修改 capability-context、eval、Actor、host/native、Cargo manifest、lockfile、DB source
语言或artifact格式。若需要上述写集外 production owner，立即停止并如实上报。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o5r-service-db
branch   codex/p5-f445h-o5r-service-db
```

先提交 implementation，再单独提交
`P5-F445H-O5R-service-db-prepared-runtime-operation-result.md`。最终 worktree clean，不
merge/rebase/push，不派子 Agent。
