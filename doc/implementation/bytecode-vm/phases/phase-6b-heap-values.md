# Phase 6B: managed heap, value semantics, ConstantHeap and recoverable parity

状态：planned；依赖Phase 6A complete

## 1. 目标

在已跑通的VM/Agine链路下完成最终物理内存模型：request GC、aggregate value semantics、ConstantHeap、
affine resources和recoverable logical-value parity。

## 2. 交付物

1. GC-capable request heap、stable generation/epoch handles、safepoints、soft/hard memory limit与完整root visitor。
2. Fiber/frame/value、PendingOperation、NativeAdapterFrame、StreamSupervisor、CleanupOwner、ResourceTable、callback
   captures、boundary buffers和TransientRootStack全部进入root/budget contract。
3. Record/Array/Map的move/share/path-COW和transient builders；dense shape/field plans与tracked edit ownership。
4. Immutable ConstantHeap和bounded thaw/materialization；constant load不产生request allocation。
5. Explicit `ValueTransferPlan`、Package-local `InOut`、move-only/affine Resource/Stream与exact/idempotent drop。
6. Recoverable codec按logical value工作：合法shared backing可重复编码，真cycle和不可恢复capability稳定拒绝。

## 3. 验收

### 3.1 Focused gate

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only skiff-tests
node scripts/verify.mjs --only test-runner
git diff --check
```

### 3.2 阶段专属证明

- allocation slow path、adapter Rust local、cross-heap partial result、Pending handoff、unwind和stream buffer均无漏root；
  debug safepoint断言transient protocol平衡。
- low-allocation request允许零GC；allocation-heavy长请求回收不可达中间值；soft limit收集后仍超hard才报结构化错误。
- aggregate copy为O(1)logical snapshot；首次write只COW对应path；unique Array/Map mutation保持目标复杂度。
- `dup`/copy/container store不能复制move-only/affine token；overwrite/frame-pop/tail/unwind/stop执行exact drop。
- `InOut`只允许Package-local verified `NoPending` path；throw不回滚已发生write；任何boundary/escape负例拒绝。
- constant load零request allocation；相同build初始化唯一frozen heap，失败不发布image。
- recoverable canonical bytes/semantic golden保持一致；物理alias不被误判为cycle，真cycle仍拒绝。

Changed-semantics测试以reference golden为准；旧evaluator结果不能作为value/const/`InOut`正确性oracle。

### 3.3 强制 Live

```bash
node scripts/verify.mjs --only router-live:agine
```

Manifest必须包含GC cycle/allocation、COW share/copy、constant load allocation、resource create/drop和Pending root
transfer计数。Chat和strict full host-tools继续全部VM、fallback=0。

## 4. 停止条件

- host adapter/raw Rust borrow跨allocation、safepoint或Pending。
- 为简化collector pin全部对象或把VM stack塞进managed heap。
- Actor heap被并入普通request collector。
- resource close依赖GC finalizer时机。
- COW共享导致普通local mutation暗中修改Actor field或另一个value snapshot。

## 5. Handoff

Phase 7必须复用本阶段heap/root/value ports，但Actor使用独立shared arena和quiescent compaction contract。
