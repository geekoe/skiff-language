# P5-F433B Remove dead WebSocket receive health instrumentation

状态：Ready。低语义Router/tooling收敛。

## 直接父节点

- `P5-F433-d4-current-residue-wave.md`

父节点冻结全部`websocketReceive`命中及其dead instrumentation性质。

## 唯一写入范围

```text
router/src/router/controlPlane.ts
router/tests/loop-risk-health.test.ts
scripts/check-loop-risk-health.mjs
scripts/lib/loop-risk-health.mjs
scripts/tests/loop-risk-health.test.mjs
scripts/tests/loop-risk-stress.test.mjs
```

以及本leaf result。禁止修改WebSocket gateway/lifecycle/dispatcher、HTTP counters、Runtime、
test-runner、其它scripts、Internals或skiff-packages。

## 必须实现

1. Router loop-risk health type、source registration和JSON snapshot删除`websocketReceive`及默认零
   shape；不存在的receive path不能继续对外承诺计数器。
2. Router tests删除相关producer/consumer/zero判断，继续精确覆盖dispatcher pending unary/stream、
   HTTP stream backpressure及runtime counters。
3. 外部health evaluator不再要求三个receive counter，也不把missing字段判错；self-test删除只靠
   `abortOnClose:1`制造的负例，并保留其它真实非零/缺失负例。
4. stress/health fixtures删除dead字段，不能用optional fallback或兼容alias保留。
5. `websocketReceive`在Router与scripts production/tests反搜为零；不得删除current
   connection lifecycle/send/generation health或HTTP/dispatcher counters。

## 验证

本Agent是以下证据的唯一owner：

```bash
pnpm --dir router test -- loop-risk-health
pnpm --dir router exec tsc --noEmit
node --test scripts/tests/loop-risk-health.test.mjs
node --test scripts/tests/loop-risk-stress.test.mjs
node scripts/check-loop-risk-health.mjs --self-test
git diff --check
```

记录Router filter真实discovery；若额外`--`导致全量，改用实际文件filter并如实记录。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f433b-receive-health`
- 分支：`codex/p5-f433b-receive-health`

启动后5分钟内实际修改。提交implementation，再新增并提交
`P5-F433B-remove-websocket-receive-health-instrumentation-result.md`。返回commit/tree、反搜、
测试discovery和clean状态。不得merge、rebase、push、stable/live或承接combined。
