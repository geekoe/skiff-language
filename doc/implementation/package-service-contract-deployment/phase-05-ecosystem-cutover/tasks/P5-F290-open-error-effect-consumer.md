# P5-F290 Open error effect consumer

状态：Ready。

## 直接父节点与权威链

- same-heap/provenance 冻结结果：
  `P5-F278-same-heap-semantic-correction-result.md`
- shared error DTO 冻结结果：
  `P5-F284-open-error-model-acceptance-result.md`
- production owner 审计：
  `P5-F280-open-service-error-channel-implementation-audit-result.md`

上述父节点继续引用唯一权威设计
`doc/architecture/package-service-contract-deployment.md`。启动时只读本任务；只有需要依据时才沿父链向上读取。

## DAG 位置与当前共享状态

- 节点：F280 `W2-E Open-channel effect consumer`。
- production base：当前 integration 已合入 F278、F281/A1、F285 与 F287。
- A1 已从 `BoundaryOperationContract` 删除 operation-specific `errors`；本任务只迁移 effect
  evaluator 的唯一旧 consumer。
- F288 正在迁移 artifact/contract consumers，但不修改本任务文件。
- 完成后解除：F286 language/source crate 的可编译恢复，以及 W2 compiler/artifact combined probe。
- 当前是实现检查点，不是稳定候选。

## 唯一写入范围

Production：

- `compiler/source/src/callable_effects/transfer/call.rs`

Test：

- `compiler/source/src/callable_effects/tests.rs`

禁止修改其它 `compiler/source` 文件、artifact/model/identity、projection、lowering、runtime、std、
router、生态仓库或任务外文档。

## 完成标准

1. `detached_contract_callee` 不再读取已经删除的 `contract.errors`，也不重建 declared throw set。
2. service contract call 始终具有开放错误通道。满足现有 detached boundary guarantees 的 unary、
   no-callback call 继续得到：
   - return origin `Fresh`；
   - throw origin `Fresh`；
   - `may_suspend` 完全沿用 contract；
   - `throws_caller_alias == false`；
   - `requires_same_heap_identity == false`。
3. `Fresh` 只表示跨 boundary 后错误 payload 已与 caller heap 分离，不表示某个错误类型集合，也不表示调用
   不会失败。
4. `detached_error` 仍是必要 guarantee。为 false 时必须 fail closed，不能生成假 `Fresh` throw；
   parameters、return、mutation、escape、same-heap、stream 或 callback 的既有 gate 均不得放宽。
5. package direct call、本地 callable、builtin receiver 与 unknown target 的 provenance 不因本任务改变。
6. F278 的真实 identity observation 语义保持；不能为了开放错误通道重新把 throw/unknown 归为
   `requiresSameHeapIdentity`。
7. 反向搜索确认该 production owner 不再引用 `BoundaryErrorContract`、`contract.errors` 或其它 closed
   error-set spelling。

## 最小风险探针与验证 owner

在现有 callable-effects fixture 中增加精确正负例，至少覆盖：

- detached service call：return/throw origins 都是 `[Fresh]`；
- `may_suspend` true/false 均只改变 suspension fact；
- `detached_error = false` 时不获得 detached callee summary；
- caller 参数、caller mutation、same-heap identity 与 throw alias 均未被错误引入；
- 一个不满足其它既有 guarantee 的 contract 仍 fail closed。

本任务唯一拥有：

```bash
cargo test -p skiff-compiler-source callable_effects -- --list
cargo test -p skiff-compiler-source callable_effects --no-fail-fast
git diff --check
```

若 source crate 被 F286/F288 尚未迁移的其它 consumer 遮挡，先用该模块的最窄可执行 test target；
记录精确遮挡，不得修改范围外文件。不得运行 workspace、完整 compiler、生态、stable、live 或 chat smoke。

## 风险、worktree 与交付

- 风险：中；验收组：后续 W2 compiler/artifact combined probe。
- worktree：`/Users/geek/workspace/skiff-p5-f290-error-effects`
- branch：`codex/p5-f290-error-effects`
- 从当前 integration HEAD 建立，不 push、不操作 stable。
- 启动到第一次 production 修改不超过 5 分钟；不可执行时立即返回
  `TASK_NOT_EXECUTABLE`、精确缺口和最小前置。
- 完成后提交并返回 commit、反向搜索、自验收矩阵和所有遮挡；不得自行承接 language/runtime 节点。
