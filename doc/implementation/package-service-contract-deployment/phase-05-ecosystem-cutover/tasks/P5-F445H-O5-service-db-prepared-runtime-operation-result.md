# P5-F445H-O5 Service-DB prepared runtime operation result

状态：`TASK_SCOPE_EXPANDED`。

本节点没有修改 production 或 tests。现有任务写集可以在 `ServiceDbRuntime` /
`ServiceDbStore` 内部拆出 prepare、无 caller heap 的 wait 和 finalize，却不能把这个协议暴露给
O6 实际持有的 `DbCapabilityStore`。因此在当前合同下继续实现只会得到一个 evaluator 无法消费的
内部 API，不能解除 O6，也不能证明 actual-Pending 的核心目标。

## 1. 输入与停止状态

| 项 | 值 |
| --- | --- |
| 直接父节点 | `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md` |
| production prerequisite | `d39ad5b0` |
| task checkpoint | `87e85911` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-o5-service-db` |
| branch | `codex/p5-f445h-o5-service-db` |
| production / test 修改 | 无 |
| stable / live / network | 未运行 |

停止原因属于公共 runtime capability seam 的 owner / 写集遗漏，不是 recoverable、encryption、
storage identity 或 DB 语言语义发生了新选择。

## 2. 精确阻塞签名

### 2.1 evaluator 只消费 capability abstraction

`runtime/eval/src/program_db.rs::execute_db_command` 的参数是
`store: &DbCapabilityStore`。recoverable 路径分别调用：

- `find_many_page_runtime`：`program_db.rs:501-509`；
- `find_one_by_key_runtime`：`program_db.rs:541-542`；
- `find_one_by_query_runtime`：`program_db.rs:546-549`；
- `create_runtime`：`program_db.rs:600-601`；
- `update_one_runtime`：`program_db.rs:643-650`；
- `replace_one_runtime`：`program_db.rs:709-716`。

`runtime/eval/Cargo.toml` 依赖 `skiff-runtime-capability-context`，不依赖
`skiff-runtime-service-db`。这保持 evaluator 与具体 Mongo/service-db provider 解耦。

### 2.2 capability trait 把 caller heap 借用绑定到整个 future

`runtime/capability-context/src/db.rs:621-743` 中的关键签名都是同一形状：

```rust
fn find_one_by_key_runtime<'a>(
    &'a self,
    type_name: &'a str,
    key: DbKey,
    projection: Option<Vec<FieldPath>>,
    heap: &'a mut RequestHeap,
    context: DbRecoverableRuntimeContext,
) -> DbCapabilityFuture<'a, Option<RuntimeValue>>;
```

`find_one_by_query_runtime`、`find_many_page_runtime`、`update_one_runtime` 和
`replace_one_runtime` 同样把 `heap: &'a mut RequestHeap` 与
`DbCapabilityFuture<'a, _>` 绑定；`create_runtime` 把
`value: &'a RuntimeValue`、`heap: &'a RequestHeap` 和 future 绑定。

`DbCapabilityStore` 只包装 `Arc<dyn DbCapabilityStoreApi>`，见
`runtime/capability-context/src/db.rs:800-821`。其公开 async 转发仍把 heap 传入旧 trait
future，例如 `find_one_by_key_runtime` 位于 `db.rs:844-855`，
`find_many_page_runtime` 位于 `db.rs:895-907`，`create_runtime` 位于
`db.rs:917-927`。`as_api()` 只返回同一个 trait object；该 trait 没有 prepared method 或
store downcast seam。

所以即使具体实现第一次 poll 后不再访问 heap，Rust 调用方仍必须认为 `RequestHeap` 在整个
future 生命周期内被借用。E3 无法在该 future 存活时同时取得 caller heap 去提交 Actor segment，
也无法在 resume 后才 finalize。

### 2.3 service-db adapter 固化了同一借用

唯一 production 实现
`runtime/service-db/src/capability.rs:138-383` 直接把 capability future 转发给
`ServiceDbStore`。例如：

```rust
fn update_one_runtime<'a>(
    &'a self,
    type_name: &'a str,
    selector: DbOneSelector,
    change: DbRuntimeChange,
    heap: &'a mut RequestHeap,
    context: DbRecoverableRuntimeContext,
) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
    Box::pin(async move {
        self.store
            .update_one_runtime(type_name, selector, change, heap, context)
            .await
            .map_err(db_capability_error)
    })
}
```

同文件的 find-one、find-many、create、replace runtime adapter 都保持这个生命周期。
`runtime/service-db/src/capability.rs` 不在 O5 授权写集内。

## 3. 为什么 O5 内部改动不能被 O6 消费

O5 原写集只能新增 `ServiceDbRuntime` / `ServiceDbStore` 的 inherent prepared methods。
这些方法无法穿过 `DbCapabilityStoreApi`：

1. evaluator 拿不到具体 `ServiceDbStore`，只拿到 type-erased `DbCapabilityStore`；
2. trait 没有 prepared method，也没有可下转到 concrete store 的 `Any` seam；
3. service-db crate 不能为 capability-context 所有的 `DbCapabilityStore` 新增 inherent method；
4. 在 service-db 中定义 extension trait也无效，因为 eval 当前不依赖该 crate，且直接依赖具体
   provider 会破坏已有 capability owner；
5. 让 O6 增加 service-db 依赖或用 global registry / unsafe 绕过 trait，既超出 O6 写集，也会把
   evaluator 绑定到 Mongo provider；
6. 保留旧 future、在内部 first poll 时 prepare不能缩短 Rust 层的 heap borrow；
7. drop 后重建 future 会重放已经开始的 DB command，违反 exactly-once 副作用合同。

因此当前任务不能通过“先实现内部 API，稍后再想办法接线”判为完成。

## 4. 推荐的最小共享接口节点

在 O5 前增加一个窄共享 checkpoint，例如
`O5A DB prepared capability seam`。该节点只拥有跨 crate 的 capability 表示，不拥有 DB
mapping、Mongo command、transaction/lease 状态机或 evaluator Actor 控制流。

建议写集：

- `runtime/capability-context/src/db.rs`
- `runtime/capability-context/src/lib.rs`（仅必要 re-export）
- `runtime/service-db/src/capability.rs`
- 上述模块现有或窄 child tests
- O5A task/result

接口必须保证：

- prepare 是同步调用；可以只在调用期间读取 caller `RuntimeValue` / `RequestHeap`；
- prepare 返回值的类型不携带 caller heap、env 或 evaluator lifetime；
- wait 是 owned、`Send` 且一次性消费，持有 provider/store、owned command/BSON、session /
  request state和必要 recoverable retention 状态；
- wait completion 携带一次性 finalize owner；finalize 只在 wait 完成并恢复 caller segment 后
  同步接收 `&mut RequestHeap`；
- wait / completion 不可 clone 后重放，drop 不执行第二次 command；
- provider error在 capability seam继续映射成既有 `DbCapabilityError`；
- 旧 async `*_runtime` API可以薄组合新协议，继续让当前调用方编译，但不能作为 O6 的
  actual-Pending 路径；
- raw `DbDocument`、transaction begin/commit/abort、claim/renew/release语义不变。

一种可行的 type-erased 形状是 capability-context 拥有：

```text
prepare_runtime(...) -> Result<PreparedDbRuntimeOperation>
PreparedDbRuntimeOperation::wait(self)
    -> Future<Output = Result<DbRuntimeFinalizer>>
DbRuntimeFinalizer::finalize(self, &mut RequestHeap)
    -> Result<runtime outcome>
```

精确 enum / trait-object 布局属于 O5A 的局部实现选择；关键合同是返回的 operation 不再带
caller heap lifetime，并且 wait 与 finalizer 都是一次性 owner。

## 5. O5A 最小测试

O5A 应先用 capability-level fake store 写 RED，至少证明：

- prepare 返回后 caller heap可立即独立 mutation，而 prepared wait仍存活；
- wait 完成前 caller heap checkpoint / stats和已有节点不变；
- wait completion 只能消费一次，drop或error不重启 provider operation；
- finalize 才接收 heap并物化结果；finalize资源失败不留下部分 allocation；
- `ServiceDbCapabilityStore` 对 find-one/find-many/create/update/replace逐项转发 prepared
  operation，并保持既有 `DbCapabilityError` 映射；
- 旧 async runtime adapter仍编译并薄组合新接口；
- raw DB、transaction和lease capability方法未改变。

聚焦命令建议：

```bash
cargo test -p skiff-runtime-capability-context prepared_db -- --nocapture
cargo test -p skiff-runtime-service-db prepared_capability -- --nocapture
cargo check -p skiff-runtime-capability-context --locked
cargo check -p skiff-runtime-service-db --locked
cargo fmt --check
git diff --check
```

## 6. 后续 DAG

```text
E3R preflight
      |
      v
O5A capability prepared seam
      |
      v
O5 service-db mapping / raw wait / finalize
      |
      v
O6 eval DB state machines
      |
      v
J1 combined owner review -> E4R
```

O5A 完成后应重发 O5 合同，把 O5A result加入直接 prerequisite，并授权
`ServiceDbRuntime` / `ServiceDbStore` 实现 capability adapter所需的 concrete prepared
operation。O6 仍只消费 capability-context 公共形状，不直接依赖 service-db。

## 7. 用户决策

当前不需要用户决策。actual-Pending、provider abstraction、storage/wire identity和
exactly-once合同都没有变化；这是父 DAG 遗漏公共接口 owner 的执行拆分问题，主 Agent可以按既有
设计新增 O5A 并重发 O5。

只有在决定让 evaluator直接依赖具体 service-db/Mongo provider、改变 DB capability公共职责，
或改变 storage/recoverable wire语义时，才需要回到用户/权威设计。本文不建议这些方向。

## 8. 验证

未运行测试或编译命令：任务在任何 production/test 修改前即按停止规则终止，动态验证没有对应候选。
仅应对本 result运行 `git diff --check` 并提交文档。
