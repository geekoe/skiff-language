# P5-F344 Router bootstrap test determinism result

状态：`PASS`（test-only 修复与验证完成；无 blocking；未修改 task 状态，未 push，未承接后续节点）。

## 候选与写入边界

- worktree：`/Users/geek/workspace/skiff-p5-f344-router-bootstrap-test`
- branch：`codex/p5-f344-router-bootstrap-test`
- task 声明起点 commit：
  `29bcb43adb6647ed87add4dfcbd2212352b96a4a`
- task 声明起点 tree：
  `b7bc3697f02f620e4c15ec480feb530189ea628c`
- worktree 启动 HEAD：
  `0bd80a4fec7bc99e72aaaa9fca070e660f8e7a5f`

`29bcb43a..0bd80a4f`只新增 F343/F344 task 文档。代码写入严格限制为
`router/tests/assembly-runtime-endpoint.test.ts`，另新增本 result；没有修改 Router production、
其它测试、protocol/corpus、配置/lockfile、设计或父 task/result。implementation/result 由同一交付
commit 承载，最终 commit 以交付消息为准。

## 精确竞态

目标测试原先先等待 WebSocket `open`，发送 capabilities 后才通过一次性 `message` listener 等待
`runtime.registered`。Router 在连接建立时发送初始 `router.bootstrap`：

- bootstrap 在 listener 建立前到达时，EventEmitter 没有缓存它，测试随后直接收到
  `runtime.registered`；
- bootstrap 在 listener 建立后到达时，同一个 listener 把它当作 register 响应，因
  `router.bootstrap !== runtime.registered`失败。

因此单 selector 的调度可以稳定表现为第一条路径，而 H/R/T 的 8 文件合流组合改变调度后稳定暴露第二条
路径。竞态属于测试消费初始连接帧的时序，不是 Router production 发送顺序或 wire contract 问题。

## 有限修复机制

只有目标 selector 的 register 响应改用本文件专用且固定等待
`runtime.registered`的`nextRuntimeRegisteredAfterInitialBootstrap`：

1. 同一个 `message` listener 持续等待可选的初始 bootstrap及随后的 register 响应，bootstrap与
   registered连续到达时没有两次 `once`之间的监听空窗。
2. 若 listener 建立前 bootstrap已到达并丢弃，首个`runtime.registered`直接成功。
3. 若 listener 建立后首帧是`router.bootstrap`，只跳过这一次并继续等待
   `runtime.registered`。
4. 任意其它首帧、第二个 bootstrap或 bootstrap后的非 registered帧仍由 exact type断言 fail
   closed；非 binary帧也立即失败。

通用`nextRuntimeFrame`保持原样；timeout仍为原有1000ms，没有 sleep、timeout扩大或 Router suite
串行化。目标测试原有 v2 control `response.error`断言保持不变，继续检查
`skiff-runtime-frame-v2`、`errorKind: control`、`InProcessServiceCallRequired`和空 payload。

## Selector 与验证

先枚举并确认目标 selector非零：

```text
tests/assembly-runtime-endpoint.test.ts >
  unified RuntimeEndpoint assembly bootstrap >
  keeps the complete generic runtime switch on the composite endpoint
```

单独运行：

```text
1 file passed
1 test passed | 9 skipped
```

随后连续5次运行 F341的7个 Router focused文件与 F342的
`tests/http-telemetry.test.ts`组成的同一8文件集合：

```text
run 1: 8 files passed / 119 tests passed
run 2: 8 files passed / 119 tests passed
run 3: 8 files passed / 119 tests passed
run 4: 8 files passed / 119 tests passed
run 5: 8 files passed / 119 tests passed
```

静态验证：

```text
pnpm --filter @skiff/router run type-check
  PASS

git diff --check
  PASS
```

worktree原本没有依赖目录；验证时只临时链接现有 Phase 5 integration worktree的完整 Router依赖，
验证后删除，不进入 diff。没有运行完整 Router/workspace/root、stable或 live验证。

## Blocking

Blocking issues：无。
