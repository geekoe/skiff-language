# Phase 4: minimal production-shaped VM vertical slice

状态：planned；依赖Phase 3B complete

## 1. 目标

打通第一个真实source -> artifact -> loader -> linker -> verifier -> VM -> response垂直切片，证明VM core和
deployment owner接口成立。本阶段不等待GC、完整boundary或Actor。

## 2. 交付物

1. 固定宽度`ValueSlot`、连续value stack、segmented `VmFrame`与同步dispatch loop。
2. Literal/slot/move-copy、算术比较、branch/switch、local/package-direct call、explicit tail call、return/throw、
   minimal constant load与hard fuel。
3. Source/call site、statement charging、stack trace、timeout/internal-stop分类的最小完整链。
4. VM通过stable handle和窄`VmHeap`接口适配现有RequestHeap；VM slot/ABI不携带`RuntimeValueCarrier`。
5. 一个真实unary service operation被标为VM-only并通过canonicalproduction-shaped ingress执行。

## 3. 非目标

- 不实现request GC、最终COW collection或Actor heap。
- 不支持VM到tree evaluator的函数/表达式fallback。
- 不因`maySuspend`选择另一套local call ABI。

## 4. 验收

### 4.1 Focused gate

```bash
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only test-runner
node scripts/verify.mjs --only skiff-tests
git diff --check
```

### 4.2 阶段专属证明

- `size_of::<ValueSlot>() == 16`及关键`VmControl`/frame layout有显式回归门禁。
- 100,000 hop tail recursion保持O(1) active frame/value/diagnostic space；每hop fuel/statement charge不丢失。
- 深non-tail recursion不增长Rust调用栈，但受VM frame/memory/fuel限制。
- 参数严格按source顺序且只求值一次；tail commit前错误仍能unwind caller frame。
- throw/catch identity、call site、source attribution和statement counts与reference golden一致。
- VM-only service的artifact/verifier/execution任一处损坏均请求失败；manifest的fallback counter为零。

### 4.3 阶段专属 Live

```bash
node scripts/verify.mjs --only router-live:http
node scripts/verify.mjs --only router-live:agine
```

`router-live:http`或其后继fixture必须把至少一个真实operation锁定为VM-only。Chat/host主链此时可以仍有legacy
deployment，但同一隔离栈必须同时证明VM canary成功和广泛回归成功。

## 5. 停止条件

- local call创建Rust future、递归进入`EvalContext`或依赖native call-depth fuse。
- `ValueSlot`只是装箱`RuntimeValueCarrier`的pointer/enum别名。
- 遇到unsupported opcode/function时回退tree evaluator。
- quickening/micro-op数量改变semantic charging或source attribution。

## 6. Handoff

Phase 5在同一`VmFiber`/frame/value/heap port上增加scheduler控制；不得另建一套async VM执行器。
