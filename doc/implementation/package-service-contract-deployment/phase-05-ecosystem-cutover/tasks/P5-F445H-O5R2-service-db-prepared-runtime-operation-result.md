# P5-F445H-O5R2 Service-DB prepared runtime operation result

状态：`IMPLEMENTATION_COMPLETE / FOCUSED_GREEN / REQUIRED_FULL_GATE_BLOCKED_BY_BASELINE`。

唯一 production `ServiceDbCapabilityStore` 已覆盖六个 prepared runtime 入口。同步 prepare 完成
metadata、selector/query/order/projection校验和 write输入的 owned BSON编码；owned wait只保存
command、document、recoverable context、retention roots、store/runtime owner等 caller-heap-free
数据；one-shot finalizer才重新接收原 caller `&mut RequestHeap`并物化结果。

六条旧 heap-borrowing runtime入口现在都是 prepare → 同一个 wait → 同一个 finalizer的薄组合。
原来分散在 capability、store和 runtime root中的第二套 runtime mapping/Mongo实现已经删除。

## 1. 输入与提交

| 项 | 值 |
| --- | --- |
| 直接父节点 | `P5-F445H-O5A-prepared-db-capability-seam-result.md` |
| 直接父节点 | `P5-F445H-O5R-service-db-prepared-runtime-operation-result.md` |
| 直接父节点 | `P5-F445H-O5B-mutable-legacy-create-finalization-seam-result.md` |
| production prerequisite | `7ac46ebe` |
| task document / worktree base | `17d260a5` |
| implementation | `91b35b05` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-o5r2-service-db` |
| branch | `codex/p5-f445h-o5r2-service-db` |

最终 production写集精确为：

- `runtime/service-db/src/capability.rs`
- `runtime/service-db/src/lib.rs`
- `runtime/service-db/src/store.rs`
- `runtime/service-db/src/prepared_runtime.rs`
- `runtime/service-db/src/prepared_runtime/{read,create,update,replace,store}.rs`

测试写集精确为：

- `runtime/service-db/src/tests.rs`，一个 child module声明和旧 direct-runtime create测试的
  mutable heap签名跟随
- `runtime/service-db/src/tests/prepared_runtime.rs`
- `runtime/service-db/src/tests/prepared_runtime/**`

没有修改 capability-context、eval、Actor、host/native、Cargo manifest、lockfile、mapping、
DB source语言或artifact格式。

## 2. Test-first 证据

先加入 concrete `ServiceDbCapabilityStore`测试，逐项要求六个同步 prepare入口成功；production
implementor尚未覆盖 O5A defaults时运行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5r2-service-db/build/cargo-target \
  cargo test -p skiff-runtime-service-db prepared_runtime -- --nocapture
```

得到预期 RED：`0/1`通过、`1/1`失败、`102` filtered。第一条
`prepare_find_one_by_key_runtime` 精确返回 O5A 的
`prepared DB runtime operation is unavailable`。测试只构造 inert URL，未 poll wait、未建立
网络连接。

完成 production后，同一聚焦 selector最终执行 `10/10` GREEN、`102` filtered。

## 3. Production 收敛

### 3.1 同步 prepare

六条 concrete prepare入口分别构造 typed one-shot operation：

- find-one by key/query：owned `MongoFindOnePlan`；
- find-many：owned `MongoFindManyPlan`，`limit: 0`保存为无需 Mongo command的 ready plan；
- create：owned storage document、recoverable roots和恢复 context；
- update：owned normalized selector、update document、wire change、cascade paths和 roots；
- replace：owned normalized selector、replacement document和 roots。

write mapping仍调用原 `DbCollectionMetadata` encryption/recoverable pipeline，但只在 prepare时读取
caller heap。prepare失败时，局部 roots和command直接丢弃，不会取得 request DB state、启动
Mongo command或持久化 root。

`Prepared*` / `Completed*` struct都不含 `RuntimeValue`、heap handle、heap index、caller heap或
evaluator引用；`RuntimeValue`只出现在同步 prepare参数或 finalizer返回类型。

### 3.2 Owned wait与 request state

wait拥有 `ServiceDbStore`和 `Arc<ServiceDbRuntime>`：

- 每次 operation在 wait开始时取得当前 request state并先检查 lease-lost；
- active transaction继续借用原 `DbRequestState.transaction.session`，session从未移出 owner；
- 无 active transaction时，read沿用无 session路径，create/update/replace沿用原自动 transaction；
- update/replace继续克隆当前 lease guards，并在同一 request state标记 lease-lost；
- retention roots、Mongo command、lease assertion、immutable-file cascade和 implicit transaction
  finish/abort顺序沿用原实现；
- unpolled drop不启动 wait；Pending drop只销毁同一个 future，不重建或重放 command。

### 3.3 Finalizer与 create

wait只返回 owned storage document。finalizer使用原 metadata、encryption和 recoverable read
context向 caller heap物化 one/many/value结果；find-many只有全部成功才返回。

create不再返回跨 wait保存的 `value.clone()`。prepare先编码 owned storage document，wait完成
insert后，finalizer从该 document恢复逻辑等价的新 heap-attached object。测试在 prepare后修改
原输入 object，最终 create结果仍保持 prepare时的字段，证明 operation未保存输入 handle。

O5A `DbRuntimeFinalizer`继续包围整个物化过程；多节点结果在第二个 allocation失败时完整恢复
finalize前 checkpoint和 stats。

### 3.4 单一核心与旧入口

capability、`ServiceDbStore`和 direct `ServiceDbRuntime`的六条旧 runtime入口都调用同一组
`prepare_*_runtime_command`、`execute`和 `finalize`。旧
`update_one_runtime_inner`、`replace_one_runtime_inner`、`create_runtime_inner`及 runtime-only
selector materializer已经删除。

capability-level旧入口通过 test-only provider driver逐项执行后，观察到与 prepared入口完全相同
的六种 wait顺序；两轮共 `12`次调用、`12`次完成，没有 legacy fallback或额外启动。

## 4. 聚焦验收矩阵

| 合同 | 测试 / 代码证据 |
| --- | --- |
| 六个 concrete prepare全部覆盖 | `concrete_provider_overrides_all_six_prepared_runtime_entries` |
| prepare失败不启动 provider | missing metadata与invalid create同步失败，driver start保持 `0` |
| caller heap在Pending期间可独立修改 | Pending wait被独立 task持有；caller继续 allocation；finalize前 checkpoint/stats不变 |
| Ready/Pending只启动一次 | ready双轮矩阵与pending gate的 start计数均精确 |
| unpolled/Pending drop不重启 | start分别为 `0`/`1`，completion为 `0`，pending drop为 `1` |
| provider error无 finalizer | error路径 completion为 `0`，caller heap不变 |
| prepared与旧入口共用核心 | 六种operation两轮 kind序列完全相同，总 start/completion均为 `12` |
| create不保存输入 handle | prepare后修改输入title，结果仍恢复prepare时title |
| recoverable/encryption在prepare编码 | create/update/replace三条write分别通过 local-interface recoverable和encrypted metadata prepare |
| 多节点finalize失败完整回滚 | 小 node budget下验证 checkpoint和stats完全恢复 |
| raw、mapping、lease/cascade helper无回归 | hermetic service-db suite除两个base限制外 `110/110` |

capability-context O5A/O5B seam selector另执行 `8/8` GREEN、`52` filtered。

## 5. 验证

所有 Cargo命令使用独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-o5r2-service-db/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-service-db prepared_runtime -- --nocapture` | PASS：`10/10`，`102` filtered |
| `cargo test -p skiff-runtime-capability-context prepared_db -- --nocapture` | PASS：`8/8`，`52` filtered |
| service-db hermetic suite，显式排除下述两个base限制 | PASS：`110/110` unit，`0` doc |
| `cargo check -p skiff-runtime-service-db -p skiff-runtime-capability-context -p skiff-runtime-eval --locked` | PASS；只有 integration既有 linker/eval warnings |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

没有运行 stable、live、network或真实 MongoDB，也没有 merge、rebase或 push。

## 6. Required full gate的两个base限制

任务同时要求完整 service-db suite和禁止真实 MongoDB。当前 base有一个未标 `ignore`的测试
`service_db_runtime_create_and_find_runtime_roundtrips_local_interface`，会直接连接
`127.0.0.1:27017`并执行drop/insert/find/replace。为遵守本节点禁令，该测试没有执行。

在只排除该真实 Mongo测试后，suite实际执行 `111`项，结果为 `110` PASS / `1` FAIL。唯一失败是
未被本节点修改的
`mongo_provider_builds_db_capability_source_from_valid_opaque_config`：

```text
service id `example.com/provider_<uuid>` projects to a character forbidden in Mongo database names
```

其 base fixture `provider_input`把 `service_id("provider")`同时写入 `service_id`和
`state_namespace`；当前 `new_with_config_and_namespace`把后者当最终 Mongo database name严格校验，
因此含 `/`和`_`的 publication id必然失败。该 fixture位于本任务只允许module声明的长
`tests.rs`，也与 prepared operation无关，本节点没有越界修改。

因此不能声称任务给出的无排除 full command GREEN。实现、focused、capability seam、联合编译和
其余 `110`项 hermetic suite均已完成；在启动 O6/J1的 required full acceptance前，需要由对应
base test owner修正这个 state-namespace fixture，并由有权运行真实 Mongo gate的 owner补齐 live
证据或把该测试归入明确的 live selector。
