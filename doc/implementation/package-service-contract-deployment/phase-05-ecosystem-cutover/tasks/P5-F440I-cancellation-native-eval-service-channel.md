# P5-F440I Cancellation native / eval / service-channel follower

状态：Ready。确定性实现 leaf；对应 F439A 冻结 DAG 的 **R1**。

## 直接父节点

- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`
- `P5-F439A-cancellation-public-surface-owner-audit-result.md`
- `P5-F440E-cancellation-runtime-terminal-checkpoint-result.md`

需要细节时只沿这三个父节点引用向上读取。

精确实现输入：

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff integration | `e01fa01d624929bd73c163fe0ec6168f91438b91` | `a75ee80da0cb9993671f512f2bb41a720527abbb` |

## 目标与唯一写集

把 capability-context 已冻结的 internal cancellation terminal贯穿 native、eval、service-to-service
error channel和driver task boundary：

- cancellation仍终止当前执行、唤醒/清理子工作，但不再 materialize 为普通 runtime payload；
- source `catch`不能观察 cancellation；
- cancellation不能编码成 service error envelope，也不能在接收侧解码为普通错误；
- deadline/instruction limit仍是可 catch、可跨 service传输的 `TimeoutError`。

唯一 production/test 写集：

- `runtime/native/**`
- `runtime/eval/**`
- `runtime/driver/**`

另可新增本 leaf result。禁止修改 capability-context、request、host、transport、runtime/model、Router、
compiler、artifact、scripts、fixtures、其它 task/result或权威设计。不得派子 agent。

## 实现合同

1. Native 与 eval 内部可以保留结构化 `Cancelled` variant和递归 `is_cancelled()` 分类，但 cancellation
   分支不能实现、构造或返回：
   - `RuntimeErrorPayload { code: "CancelError" }`；
   - `CatchIdentity` / `PlatformBuiltinErrorIdentity::Cancel`；
   - `RequestException`、identified runtime value或其它可被 `ExprIr::Catch` 匹配的对象；
   - `ServiceErrorEnvelope`、`InternalError`或`ProviderUnavailableError`降级结果。
2. 现有 ordinary errors继续通过明确的 ordinary projection API产生 payload/catch；内部 terminal必须在
   projection/materialization之前分流。不得用 wildcard、opaque boxing或字符串 code重新混合两类结果。
3. `runtime/eval/src/exceptions.rs`及其 caller只为 ordinary throw/error构造 request exception。
   Stale Cancel builtin/type仍 fail closed；不新增 compatibility identity。
4. Service error channel：
   - provider export cancellation时返回独立 terminal outcome，不编码任何 envelope/frame；
   - caller import端不能把 cancellation当成远端业务错误；
   - unary与stream路径都必须在 serialization前短路；
   - channel API本身 fail closed，不能只依赖某个当前 caller 的 `is_cancelled()` guard。
5. `async_stream_cancel` 保持 biased winner、losing lane cancel、stream cleanup和single terminal；
   cancel/provider completion、consumer break/provider error同时 ready时不能产生晚到普通错误。
6. Actor cancellation走同一 internal terminal。Actor deadline不得再发 raw `DeadlineExceeded`，
   必须保持普通 `TimeoutError`及对应catch/service projection。
7. Driver跨 task boundary必须结构化保留 terminal，不能因为 join/boxing转成普通 wire error。
8. Native host operation的取消继续尽快 abort future/stream；本任务只处理 native/eval侧结果分类，
   request/Host/transport的最终response suppression归R2。

## 测试先行

先落真实 red，至少覆盖：

1. service channel cancellation export产生“terminal、无 envelope”；
2. eval cancellation无法形成 request exception，普通 `catch`不能捕获；
3. native/eval cancellation无 payload/catch projection，递归 wrapper仍能识别 terminal；
4. in-process unary provider cancellation不 export/import；
5. stream provider cancellation、consumer break与 losing lane cleanup；
6. actor cancellation与actor deadline分别为 internal terminal / `TimeoutError`；
7. driver task boundary保留 cancellation terminal；
8. cancel、deadline、provider completion同时 ready的确定性winner与single terminal。

必须保留/增加正例，证明 deadline和instruction limit：

- code仍为 `TimeoutError`；
- catch identity仍为 Timeout；
- service envelope round-trip仍成功。

## 验证

先从实际 crate/test inventory确定非零 selectors，再运行受影响的 focused matrix，至少包括：

```bash
cargo test -p skiff-runtime-native
cargo test -p skiff-runtime-eval
cargo test -p skiff-runtime-driver
cargo check -p skiff-runtime-native
cargo check -p skiff-runtime-eval
cargo check -p skiff-runtime-driver
cargo fmt --all -- --check
git diff --check
```

若完整 crate成本明显高，可先跑精确 selectors；交付前仍须运行上述三个 crate 的完整测试，除非存在明确的
R2/M0外部 compile blocker，并在 result中给出文件、错误和owner。

反向搜索本写集 production：

```bash
rg -n 'CancelError|PlatformBuiltinErrorIdentity::Cancel' runtime/native runtime/eval runtime/driver
rg -n 'Cancelled|is_cancelled|catch_projection|ServiceErrorEnvelope' runtime/native runtime/eval runtime/driver
```

第一条production应为零；命名清楚的 legacy rejection/negative fixture可保留并逐项分类。第二条所有
cancellation carrier、guard和channel入口必须逐 owner说明。

## 停止规则与 R2 交接

- capability-context新 API不足、或必须先改 runtime/model 公共 enum，返回 `TASK_SCOPE_EXPANDED`，不得越界。
- request/Host/transport因本任务删除 ordinary projection出现的 compile break是预期 R2 blocker；精确记录，
  不得在本 leaf修复。
- 不运行完整 verify、Router、live、instance或stable。

Result必须列出：

- red/green tests和通过计数；
- cancellation没有 envelope/exception/payload的直接证据；
- Timeout正例；
- R2与runtime/model的精确剩余 consumer；
- reverse-search分类和clean状态。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f440i-cancellation-native-eval`
- branch：`codex/p5-f440i-cancellation-native-eval`
- result：`P5-F440I-cancellation-native-eval-service-channel-result.md`

Implementation 与 result 分开提交。不 merge/rebase/push。
