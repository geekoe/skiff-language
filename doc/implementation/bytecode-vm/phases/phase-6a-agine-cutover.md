# Phase 6A: boundary, DB/unwind, callback and Agine VM cutover

状态：planned；依赖Phase 5 complete

## 1. 目标

补齐真实Agine/AIHub/Codex Relay链路需要的boundary、DB、异常、timeout、host effect和same-Runtime callback，
然后把该闭包全部切到VM。此后所有内存工作都在真实VM Live链路上继续。

## 2. 交付物

1. Service/owner boundary使用typed plans把参数、结果、错误和stream item materialize到destination heap。
2. Exception regions、resumable `UnwindState`、timeout/internal-stop、transaction Body/Commit/Abort和resource cleanup。
3. DB atomicity只覆盖driver transaction；ordinary local/Actor write不建立heap rollback。
4. Same-Runtime callback capability通过owner lookup + `EnterChild`执行并pinexact owner build/lifetime。
5. Cross-Runtime callback placement在deployment admission稳定拒绝；不新增Router reverse callback transport。
6. Agine、AIHub、Codex Relay及本轮chat/host所需Package/deployment全部发布VM artifact并只走VM。

## 3. 验收

### 3.1 Focused gate

```bash
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only router
node scripts/verify.mjs --only test-runner
node scripts/verify.mjs --only skiff-tests
git diff --check
```

`internals/agine`、AIHub和Codex Relay的相关type-check/service tests也必须在同一三仓候选上通过。

### 3.2 阶段专属证明

- caller/provider heap/frame/loan/raw handle不跨boundary；success/error/stream item使用同一typed plan owner。
- provider pointer更新保持in-flight owner pin并只影响新invocation。
- commit success、commit failure后best-effort abort、body throw/abort failure、timeout/stop late completion均保持唯一
  visible terminal和exactly-once cleanup。
- same-Runtime callback成功、owner exit/cancel/lifetime expiry后稳定失败；跨Runtime placement在执行前拒绝。
- callback targeted fixture真实执行`CallbackCapability` carrier；Agine package-local callback + forward AIHub call不算该证据。
- manifest中chat/host closure每个deployment的engine均为VM，legacy/fallback计数为零。

### 3.3 强制 Live

```bash
node scripts/verify.mjs --only runtime-live \
  --runtime-live-config <isolated-runtime-config> \
  --runtime-live-reload-url <isolated-control-url> \
  --runtime-live-artifact-root <isolated-artifact-root>
node scripts/verify.mjs --only router-live:agine
```

`runtime-live`的三个target参数必须来自组合harness manifest，不能猜stable端口。Chat必须满足canonical smoke reply；
full host-tools必须completed、非空并产生允许的file tool call。两者全部deployment VM证据是本阶段硬门禁。

## 4. 停止条件

- live closure任一deployment仍依赖tree evaluator、legacy reader或assembly generation。
- callback跨Runtime时退化为普通forward service call或本地method table。
- transaction/unwind通过RequestHeap checkpoint回滚ordinary memory write。
- timeout/stop后late result写回已结束request heap。

## 5. Handoff

Phase 6B从本阶段全VM Live基线开始；修改heap/value语义后必须重跑同一chat/host closure，不能退回legacy作比较。
