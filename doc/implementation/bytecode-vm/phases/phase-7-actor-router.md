# Phase 7: Actor and Router exact-build lifecycle

状态：planned；依赖Phase 6B complete

## 1. 目标

让Actor live incarnation、Router owner lease、Runtime shared arena、continuation和durable task都服从exact build
identity，同时保持Actor同步写立即可见、失败不回滚、只有actual Pending释放lease的语义。

## 2. 交付物

1. Actor逻辑identity继续是type + key/id；live incarnation另pin exact deployment buildId、image、implementation
   identity、fence、arena epoch和state heap。
2. Same Actor id请求不同build稳定拒绝，不升级、不替换heap、不刷新idle/lease clock。
3. 不同Actor ids可以同时pin不同build；旧incarnation真实销毁后，新claimant可使用new/same/rollback任意exact build。
4. Router/Runtime `IdleEvict`使用request/ack/fence顺序；Router lease expiry不被当成Runtime memory已销毁的证明。
5. Actor VM fiber直接使用shared arena；同步段保持lease，actual Pending时release，resume前reacquire并校验fence/epoch。
6. Durable task冻结exact build和activation snapshot；不把已逐出的in-memory Actor field当作durable state恢复。

## 3. 验收

### 3.1 Focused gate

```bash
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only router
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only skiff-tests
node scripts/verify.mjs --only test-runner
git diff --check
```

### 3.2 阶段专属证明

必须覆盖以下race和状态序列：

- same id/build A已live时，build B claim拒绝且A的idle/lease deadline不变化；
- id1/build A与id2/build B并存，调用和telemetry各自归因到exact image；
- idle request发出后Router在ack前不清fence；Runtime crash/disconnect/late ack和owner lease expiry不会形成双owner；
- incarnation真实destroy后，next claimant分别用newer、same和rollback build创建成功；旧heap/continuation不可复活；
- Actor Ready调用不释放lease；Pending释放后另一个method可以观察已发生write，resume重新校验并观察并发write；
- stale build/fence/epoch/cancel continuation拒绝且不重装lease；
- return、throw、timeout/internal failure均不回滚已执行Actor write；DB transaction只回滚DB driver write；
- quiescent compaction只在无active/suspended continuation时运行并bump epoch；
- durable task的exact build、retry/lease recovery和snapshot facts在Router restart/Runtime replacement后保持。

### 3.3 阶段专属 Live

```bash
node scripts/verify.mjs --only router-live:actor
node scripts/verify.mjs --only durable-task-e2e-live
node scripts/verify.mjs --only router-live:agine
```

Actor和durable harness必须使用真实compiler artifact、真实Router/Runtime、至少两个runtime/replica场景，并在
manifest中记录build/fence/epoch/owner transitions。Chat和strict full host-tools继续全部VM、fallback=0。

## 4. 停止条件

- same-id build mismatch触发隐式upgrade、heap迁移或idle refresh。
- Router仅凭TTL/连接断开就允许新owner，而旧Runtime可能仍持有arena。
- Actor invocation把field graph clone到request heap再commit/rollback。
- service/callback同步child被伪造成Pending并过早释放Actor lease。
- durable retry从current release pointer漂移到非冻结build。

## 5. Handoff

Phase 8只做全入口切换和删除；不得在那里首次补Actor exact-build或race语义。
