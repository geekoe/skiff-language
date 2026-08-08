# Phase 5: scheduler trampoline, native adapters and streams

状态：planned；依赖Phase 4 complete

## 1. 目标

让VM只在本次具体操作实际返回`Pending`时park，并以flat scheduler trampoline执行child、native adapter和stream。
本阶段结束时，真实chat至少穿过一个VM-only provider deployment。

## 2. 交付物

1. `Continue/Complete/EnterChild/EnterAdapter/EmitStream/Park`控制协议和唯一resume descriptor。
2. `Ready`/`ReadyError`同步返回不发布Pending、不释放heap/lease；actual Pending才原子转移roots/budget/resources。
3. Completion cell覆盖complete-before-register、cancel/deadline race、duplicate completion和single wake/claim。
4. Resumable native adapter frame通过`EnterChild`调用restricted callback，不递归poll VM。
5. Stream supervisor、affine endpoint、bounded buffer/backpressure、end/error/cancel和owner pin。
6. 一个chat参与的真实provider deployment切为VM-only；同一阶段内不得切回legacy。

## 3. 验收

### 3.1 Focused gate

```bash
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only router
node scripts/verify.mjs --only test-runner
node scripts/verify.mjs --only skiff-tests
git diff --check
```

### 3.2 阶段专属证明

- Ready success/error的park、wake、lease-release和continuation allocation计数均为零。
- actual Pending恰好publish/park/wake/claim/resume一次；precompleted和cancel/deadline竞争只有一个winner。
- 深层service/native callback child交替调用不增长Rust栈；child同步完成立即回parent。
- Pending前roots从fiber/transient owner原子转到pending owner，恢复或取消后exactly-once归还/销毁。
- stream buffer满才形成真实backpressure Pending；item跨heap materialize；consumer drop使producer最终停止且late item
  不写回结束heap。
- manifest证明chat request经过指定VM provider，并给出VM dispatch、Ready、Pending、Park/Resume和fallback=0计数。

### 3.3 阶段专属 Live

```bash
node scripts/verify.mjs --only router-live:http
node scripts/verify.mjs --only router-live:ws
node scripts/verify.mjs --only router-live:agine
```

Chat和full host-tools都必须成功；其中chat必须有VM provider证据。host-tools尚不能单独证明same-Runtime callback
capability或RemoteBoundary，这些另由Phase 6A fixture验收。

## 4. 停止条件

- 静态`maySuspend`直接造成yield/Park或不同local call convention。
- adapter在Rust栈上递归执行VM callback。
- Pending state保存可能移动的raw pointer/borrow。
- 为通过chat把已迁移provider临时改回legacy。

## 5. Handoff

Phase 6A复用同一trampoline处理service/Actor/callback boundary和async unwind，不得增加第二个scheduler。
