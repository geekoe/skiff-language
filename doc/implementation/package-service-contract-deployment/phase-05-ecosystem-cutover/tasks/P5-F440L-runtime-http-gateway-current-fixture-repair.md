# P5-F440L Runtime HTTP gateway current fixture repair

状态：Ready。确定性单文件 test repair。

## 直接父节点

- `P5-F440B-bidirectional-websocket-owner-audit-result.md`
- `P5-F440I-cancellation-native-eval-service-channel-result.md`

两个父节点都独立复现同一固定输入阻塞：

```text
runtime/eval/src/runtime_http_gateway/tests.rs:384
Option<PackageCallableId> 没有 as_str()
```

精确实现输入：

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff integration | `1897df3f98c6ba1d4f8522c5003e295380ed54e3` | `483ba35924a46bf6e08052c6fb4a7c61e113535d` |

## 唯一目标与写集

把 HTTP gateway测试 fixture适配到当前 `DeploymentGatewayEntry.handler:
Option<PackageCallableId>`：

- HTTP callable entry必须显式要求 handler存在，再把 exact callable id交给 fixture resolver；
- 缺少 handler时测试应以明确 invariant message失败，不能 fallback到selector、空字符串或其它 callable；
- production、artifact schema、compiler、fixture authoring和现有断言语义均不变。

唯一代码写集：

- `runtime/eval/src/runtime_http_gateway/tests.rs`

另可新增本 leaf result。禁止修改任何 production、其它 test、task/result或权威设计。不得派子 agent。

## 验证

先原样运行并记录当前 red：

```bash
cargo test -p skiff-runtime-eval --lib
```

修复后运行：

```bash
cargo test -p skiff-runtime-eval --lib
cargo test -p skiff-runtime-eval --test catch_fixture_closure
cargo check -p skiff-runtime-eval
cargo fmt --all -- --check
git diff --check
```

Result列出 red首错、green计数、精确 diff和clean状态。若必须改 production或其它 fixture，返回
`TASK_SCOPE_EXPANDED`，不要扩张。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f440l-runtime-http-test-repair`
- branch：`codex/p5-f440l-runtime-http-test-repair`
- result：`P5-F440L-runtime-http-gateway-current-fixture-repair-result.md`

Implementation 与 result 分开提交。不 merge/rebase/push。
