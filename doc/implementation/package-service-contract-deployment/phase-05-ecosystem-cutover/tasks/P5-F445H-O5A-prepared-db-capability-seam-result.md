# P5-F445H-O5A Prepared DB capability seam result

状态：`IMPLEMENTATION_COMPLETE / CAPABILITY_GREEN / STRUCTURE_CORRECTION_GREEN`。

O5A 已在 capability abstraction 中建立一次性 prepared DB runtime operation seam。prepare 是同步
调用，返回值只持有 `'static + Send` owned wait；wait 完成后返回一次性 `FnOnce` finalizer，
finalizer 才同步接收 caller `&mut RequestHeap`。find-one / update / replace、find-many 和 create
分别使用 `Option<RuntimeValue>`、`Vec<RuntimeValue>`、`RuntimeValue` 三种静态结果类型，不通过
一个可混淆的 runtime outcome enum 传递。

当前唯一 service-db implementor 尚未覆盖新 seam。trait 默认实现稳定返回
`prepared DB runtime operation is unavailable`，并且不会调用任何旧的 heap-borrowing async
runtime 方法。因此本 checkpoint 可编译但明确 fail closed；O5R 必须逐项覆盖六个 prepare 方法，
O6 才能把它当作 production path。

## 1. 输入与提交

| 项 | 值 |
| --- | --- |
| 直接父节点 | `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md` |
| 直接父节点 | `P5-F445H-O5-service-db-prepared-runtime-operation-result.md` |
| production prerequisite | `b6cb8a5d` |
| task document | `1395c8e6` |
| implementation | `bcda3eb2` |
| initial result | `447e445d` |
| test structure correction | `3551537a` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-o5a-db-capability` |
| branch | `codex/p5-f445h-o5a-db-capability` |

最终代码写集精确为：

- `runtime/capability-context/src/db.rs`
- `runtime/capability-context/src/db/prepared_runtime.rs`
- `runtime/capability-context/src/db/prepared_runtime_tests.rs`
- `runtime/capability-context/src/db/prepared_runtime_tests/**`
- `runtime/capability-context/src/lib.rs`，仅公共 re-export

没有修改 service-db、eval、Actor、host/native、Cargo manifest 或 lockfile。

### 1.1 文件职责

production职责保持三层：

- `db.rs` 只拥有 trait default与 `DbCapabilityStore` forwarding；
- `db/prepared_runtime.rs` 只拥有 one-shot operation、owned wait、finalizer和 typed aliases；
- `lib.rs` 只 re-export新公共类型。

初始 test fixture集中在一个 1174 行文件。`3551537a` 只做结构修正，没有修改 production API、
行为或七个测试的断言：

| 测试文件 | 职责 | 行数 |
| --- | --- | ---: |
| `prepared_runtime_tests.rs` | child module目录 | 3 |
| `prepared_runtime_tests/contract_tests.rs` | typed/default/raw合同测试 | 210 |
| `prepared_runtime_tests/lifecycle_tests.rs` | Pending、drop/error、rollback生命周期测试 | 193 |
| `prepared_runtime_tests/fake_store.rs` | fake facade与构造入口 | 26 |
| `prepared_runtime_tests/fake_store/prepared.rs` | prepared/default fake implementors | 162 |
| `prepared_runtime_tests/fake_store/state.rs` | 计数、gate、runtime context与lease hold | 136 |
| `prepared_runtime_tests/fake_store/raw_read_api.rs` | raw read/transaction/lease trait适配 | 197 |
| `prepared_runtime_tests/fake_store/raw_write_api.rs` | raw write trait适配 | 90 |

因此没有新的测试文件继续混合全部职责，最大文件为 210 行。

## 2. Test-first 证据

先加入 capability-level fake store 与七个 `prepared_db` 测试，再运行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5a-db-capability/build/cargo-target \
  cargo test -p skiff-runtime-capability-context prepared_db -- --nocapture
```

结果为预期 RED，exit `101`，共报告 39 个编译错误：

- 六个 `prepare_*_runtime` 不是 `DbCapabilityStoreApi` 成员；
- `PreparedDbRuntimeOperation`、typed aliases 和 `DbRuntimeFinalizer` 不存在；
- `DbCapabilityStore` 没有六个同步 prepare forwarding methods。

该 RED 来自 capability seam 本身缺失，不来自 service-db、evaluator、旧 exhaustive match 或
测试环境。

随后才加入 production types、trait defaults、store forwarding 和 public re-export。同一聚焦
命令转绿，实际执行 `7/7` tests。结构修正后再次执行同一命令，测试名称、数量与断言保持
`7/7` GREEN。

## 3. 一次性 ownership 合同

公共形状为：

```text
PreparedDbRuntimeOperation<T>
  .into_wait()
  -> Future<Output = Result<DbRuntimeFinalizer<T>>>

DbRuntimeFinalizer<T>
  .finalize(self, &mut RequestHeap)
  -> Result<T>
```

`PreparedDbRuntimeOperation<T>`：

- 内部 wait 是 `Future + Send + 'static`，不能携带 caller heap、env 或 evaluator borrow；
- `into_wait(self)` 消费 operation；
- 没有 `Clone`，也没有从 borrowed legacy future构造的 fallback；
- drop 只 drop 同一个 owned future，不重建、不重放 provider operation。

`DbRuntimeFinalizer<T>`：

- 内部是 `FnOnce(&mut RequestHeap)`，`finalize(self, ...)` 消费 finalizer；
- wait 结束前完全拿不到 caller heap；
- finalize 前记录 heap checkpoint；
- materialization返回资源错误时 truncate本次新增节点并恢复原 stats；
- provider finalizer不得在仍可能报错时原地修改 checkpoint之前的既有节点；prepared DB
  materialization只应新增结果节点。

编译期 typed aliases固定为：

| 路径 | prepared result |
| --- | --- |
| find-one by key/query | `PreparedDbRuntimeOperation<Option<RuntimeValue>>` |
| update-one / replace-one | `PreparedDbRuntimeOperation<Option<RuntimeValue>>` |
| find-many page | `PreparedDbRuntimeOperation<Vec<RuntimeValue>>` |
| create | `PreparedDbRuntimeOperation<RuntimeValue>` |

因此 one / many / value不能在 finalize 后靠错误 enum branch 或 downcast 混用。

## 4. Trait 与默认 fail-closed

`DbCapabilityStoreApi` 和 `DbCapabilityStore` 都新增六个同步入口：

- `prepare_find_one_by_key_runtime`
- `prepare_find_one_by_query_runtime`
- `prepare_find_many_page_runtime`
- `prepare_create_runtime`
- `prepare_update_one_runtime`
- `prepare_replace_one_runtime`

trait 默认实现只构造既有 `DbCapabilityError::ProviderUnavailable`，固定 target 为 `serviceDb`、reason
为 `prepared DB runtime operation is unavailable`。默认实现不引用、poll 或重建旧
`find_*_runtime` / `create_runtime` / `update_one_runtime` / `replace_one_runtime` future。

六个旧 async runtime methods原样保留，raw `DbDocument` methods、transaction begin/commit/abort、
lease claim/renew/release/read以及 file-record capability signatures均无 diff。当前
`ServiceDbCapabilityStore` 因默认方法继续编译，但实际调用 prepared path会 fail closed，符合本
checkpoint 的分阶段合同。

## 5. 自验收矩阵

| 任务合同 | production 证据 | fake test 证据 |
| --- | --- | --- |
| prepare 后立即释放 caller heap borrow | prepare 返回 `'static` owned wait | `pending_wait_releases_caller_heap_until_finalize` 在 wait存活时继续 allocation |
| wait 前后不改 caller heap | wait类型没有 heap lifetime；只有 finalizer接收 heap | 同一测试比较 checkpoint、stats、len和既有节点 |
| Ready / Pending 都只启动一次 | one-shot boxed future、`into_wait(self)` | `ready_and_pending_waits_start_once` |
| drop/error不重启 | 无 restart/drop fallback，future单 owner | `drop_and_error_do_not_restart_wait_or_finalize` |
| finalizer一次消费 | `FnOnce` + `finalize(self, ...)`，无 Clone | drop completion不执行 finalize；成功路径计数恰为一次 |
| finalize资源失败回滚 allocation | checkpoint + `rollback_to_checkpoint` | 小 `max_nodes` heap中第一笔成功、第二笔失败后 checkpoint/stats完全恢复 |
| one/many/value不混淆 | 三个 concrete generic aliases | 六条 eval可达 runtime path都有显式 Rust结果类型 |
| 默认 fail closed且不调用旧 runtime | 固定 provider-unavailable default | 六个默认入口全部失败，旧 runtime method计数保持 `0` |
| raw/transaction/lease不回归 | 既有 trait和wrapper签名未改 | begin/find/claim/read/release/commit/abort forwarding仍按原顺序执行 |

## 6. 验证

所有 Cargo 命令使用 worktree 内独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-o5a-db-capability/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-capability-context prepared_db -- --nocapture` | PASS：`7/7` focused tests |
| `cargo test -p skiff-runtime-capability-context --locked --no-fail-fast` | PASS：`59/59` unit tests，`2/2` doc tests |
| `cargo check -p skiff-runtime-capability-context --locked` | PASS |
| `cargo check -p skiff-runtime-service-db --locked` | PASS；证明当前唯一旧 implementor通过默认 seam继续编译 |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

`3551537a` 结构修正后的最终树重新执行了 focused、完整 capability-context、fmt和diff四项 gate；
`59/59 + 2/2` 数量没有变化。两项 `cargo check` 来自 production implementation树，而
`bcda3eb2..3551537a` 的 production diff为空。

反向 scope 检查：

```text
git diff --exit-code 1395c8e6..3551537a -- \
  runtime/service-db runtime/eval Cargo.toml Cargo.lock \
  runtime/capability-context/Cargo.toml

git diff --exit-code bcda3eb2..3551537a -- \
  runtime/capability-context/src/db.rs \
  runtime/capability-context/src/db/prepared_runtime.rs \
  runtime/capability-context/src/lib.rs
```

两项结果均为空。最终 candidate只包含任务允许的 capability与测试路径；没有运行
stable、live、network，也没有 merge、rebase 或 push。

## 7. 后继

O5R 可直接在 service-db owner 内覆盖六个 prepare methods：

1. prepare同步完成 runtime value / recoverable command编码；
2. `PreparedDbRuntimeOperation::new` 持有 provider-owned command/request状态；
3. wait只返回 `DbRuntimeFinalizer::new(...)`；
4. finalizer在 caller resume 后物化 typed value；
5. 不调用本节点保留的旧 heap-borrowing async runtime方法作为 prepared fallback。

O5R 完成后，O6 才能消费这些 methods。transaction与lease多阶段状态机仍属于 O6，不进入本节点。
当前没有需要用户决定的语言、storage、recoverable或错误语义。
