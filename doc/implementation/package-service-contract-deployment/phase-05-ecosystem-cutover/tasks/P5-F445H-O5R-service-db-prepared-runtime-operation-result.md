# P5-F445H-O5R Service-DB prepared runtime operation result

状态：`TASK_SCOPE_EXPANDED`。

本节点没有保留 production 或 test 修改。六条 concrete prepared provider 路径本身可以在
service-db 写集内拆成同步 prepare、owned wait 和 resume 后 finalizer；但 O5A 保留的旧
`create_runtime` capability 签名只有共享 `&RequestHeap`，无法按本任务要求薄组合到同一个接收
`&mut RequestHeap` 的 `DbRuntimeFinalizer`。修正该公共 capability 签名不在 O5R 写集内。

## 1. 输入与停止状态

| 项 | 值 |
| --- | --- |
| 直接父节点 | `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md` |
| 直接父节点 | `P5-F445H-O5-service-db-prepared-runtime-operation-result.md` |
| 直接父节点 | `P5-F445H-O5A-prepared-db-capability-seam-result.md` |
| production prerequisite | `c0f68ce0` |
| task document / worktree base | `70372af0` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-o5r-service-db` |
| branch | `codex/p5-f445h-o5r-service-db` |
| production / retained test 修改 | 无 |
| stable / live / network / real MongoDB | 未运行 |

停止原因是 O5A compatibility seam 的可变性遗漏，不是 Mongo、recoverable、encryption、
transaction、lease 或 file cascade 语义发生了新选择。

## 2. 精确阻塞签名

### 2.1 旧 create runtime 只有共享 heap

`runtime/capability-context/src/db.rs` 中：

```rust
fn create_runtime<'a>(
    &'a self,
    type_name: &'a str,
    value: &'a RuntimeValue,
    heap: &'a RequestHeap,
    context: DbRecoverableRuntimeContext,
) -> DbCapabilityFuture<'a, RuntimeValue>;
```

`DbCapabilityStore::create_runtime`、唯一 production implementor
`ServiceDbCapabilityStore::create_runtime`、`ServiceDbStore::create_runtime` 和
`ServiceDbRuntime::create_runtime` 都继续只接收 `&RequestHeap`。

当前实现因此可以在 Mongo wait 后直接返回 `value.clone()`。若 `value` 是 object、array、
interface 或其它 heap-backed value，这个 clone 仍是 caller heap handle；旧 future 会让该
handle及 heap borrow跨整个 wait 存活。

### 2.2 prepared create finalizer 必须可变访问原 caller heap

同一 trait 的新入口已冻结为：

```rust
fn prepare_create_runtime(
    &self,
    type_name: &str,
    value: &RuntimeValue,
    heap: &mut RequestHeap,
    context: DbRecoverableRuntimeContext,
) -> DbCapabilityResult<PreparedDbValueRuntimeOperation>;
```

O5A 的 completion 只有这一种 finalize 形状：

```rust
DbRuntimeFinalizer::finalize(self, heap: &mut RequestHeap)
```

这是必要的：prepared create 在 prepare 时可以把输入安全编码成 owned BSON/document；wait
完成后若要从该 owned 表示恢复 runtime object，必须向原 caller heap分配结果节点。共享
`&RequestHeap` 无法安全地产生该 `&mut RequestHeap`。

因此旧入口无法满足本任务冻结的唯一核心：

```text
prepare -> same owned wait -> same DbRuntimeFinalizer -> caller heap
```

## 3. 排除的规避方向

| 方向 | 问题 |
| --- | --- |
| prepared operation 保存原 `RuntimeValue` | heap-backed value仍是 caller handle，直接违反 wait不得引用 caller heap |
| 旧 wrapper在 wait后直接返回 `value.clone()` | 没有消费同一个 finalizer，保留第二套 create完成语义，并继续让 handle跨 wait |
| clone caller heap后调用 finalizer | finalizer新增节点只存在于临时 clone；返回 handle在原 caller heap中不存在，clone drop后也没有 owner |
| 在临时 heap解码后深拷贝回 caller | 最后一步仍需要原 caller `&mut RequestHeap`，旧签名没有该权限 |
| 扫描 caller heap复用“相等对象” | 等值对象可能有多个，alias/identity不确定，且仍不能为任意新结果分配 |
| `unsafe`把共享引用转成可变引用 | 违反 Rust alias规则和任务明确禁令 |
| global registry / heap mutex / eval直连 service-db | 超出 owner边界，并回到 E3R 已排除的 heap别名方案 |
| prepared入口回调旧 async create | 重新引入 heap-borrowing wait，不能解除 O6 actual-Pending |

其它五类返回值都已经拥有可变 caller heap的旧签名；阻塞只来自 create 的 compatibility 签名，
不是 prepared protocol本身不可实现。

## 4. 推荐最小修正节点

在 O5R 前增加一个 capability-context owner checkpoint，例如
`O5B mutable legacy create finalization seam`：

1. 把 `DbCapabilityStoreApi::create_runtime` 和 `DbCapabilityStore::create_runtime` 的 heap参数改为
   `&mut RequestHeap`；
2. 更新唯一 production implementor和 capability-level fake implementors；
3. 增加编译/行为测试，证明旧入口可以执行
   `prepare_create_runtime -> into_wait -> DbRuntimeFinalizer::finalize(heap)`；
4. raw create、prepared create、其它五条 runtime方法、transaction/lease签名保持不变；
5. 不增加 overload、dual path或 compatibility fallback。

Skiff 尚未发布，因此这是公共 Rust capability seam 的直接收敛，不需要保留旧 immutable
signature。O5B 完成后重发 O5R；service-db owner即可让六条旧 runtime入口和六条 prepared入口
共用一套 mapping、Mongo状态机和 finalizer。

若产品要求 `db.create(value)` 的返回值必须与输入保持同一 heap object identity，则 owned wait
模型本身无法同时满足该要求；但当前 O5R 合同已经明确允许从 prepare生成的 owned
business/storage表示恢复逻辑等价返回值，因此本 result不把它重新打开为用户决策。

## 5. Test-first 与验证

曾先加入一个临时 focused RED，构造 concrete `ServiceDbCapabilityStore` 并要求六个
`prepare_*_runtime` 同步成功。运行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5r-service-db/build/cargo-target \
  cargo test -p skiff-runtime-service-db prepared_runtime -- --nocapture
```

结果为预期 RED：`0/1` 通过、`1/1` 失败、`102` filtered；第一条 concrete
`prepare_find_one_by_key_runtime` 即落入 O5A 默认 unavailable。fixture只构造 inert Mongo URL，
未 poll wait、未建立网络连接。

在实现探索到 create签名阻塞后，按停止规则撤回该临时测试和全部探索性 production改动。最终
候选只包含本 result。最终执行：

```text
git diff --check
```

结果为 PASS。未运行 full service-db suite，因为不存在可验收 production candidate，且任务要求
停止而不是用第二套 create路径制造假 GREEN。
