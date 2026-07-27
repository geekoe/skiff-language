# P5-F442D Source-layout checker closeout

状态：Ready。把default source-layout checker收敛到current builtin/std surface，关闭K节点。

## 直接父节点

- `P5-F442A-final-fixture-tooling-preflight-result.md`

父审计实测checker当前错误要求已删除public builtin `CancelError`，且漏检current `Actor`、
`std.service.InternalError`、WebSocket与HTTP surface。实现基线为 `0303fe5d`。

## 唯一写集

- `scripts/check-skiff-source-layout.mjs`
- 本节点result

不得修改compiler registry、prelude/std source、`std/api.yml`、其它checker/test、README或
production。

## 要求

以current `compiler/core/src/prelude_registry.rs`、`std/api.yml`与对应`std/*.skiff`为事实源：

- builtin inventory正向要求`Actor`；
-继续拒绝`ActorRef`；
-删除`CancelError`正向要求并新增明确负向拒绝；
-要求`std.service.InternalError`；
-覆盖current WebSocket四个types、四个direct send native、
  `requestJsonToConnection` native和两个JSON source helpers；
-覆盖current HTTP公开types/native/source helper；
-保留current file inventory；
-不得重新引入第二套service contract或obsolete receive/sendJson旧名。

若source/api彼此不一致，停止并返回 `TASK_SCOPE_EXPANDED`，不要让checker自行选择。

## Test-first与验证

先执行当前checker记录真实CancelError RED，再修改：

```bash
node scripts/check-skiff-source-layout.mjs
node scripts/verify.mjs --only checks --list
git diff --check
```

若verify CLI的list形式与任务不同，只读确认usage后使用等价只列命令并记录。不得运行stable、
network、live或完整checks。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f442d-source-layout-checker`
- branch：`codex/p5-f442d-source-layout-checker`
- result：`P5-F442D-source-layout-checker-closeout-result.md`

Implementation与result分开提交。5分钟内开始修改；不得派子Agent，不得merge/rebase/push。
