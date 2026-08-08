# Phase 0: baseline ledger and trustworthy Live foundation

状态：planned

## 1. 目标

把 [`../README.md`](../README.md) 冻结的全部 hunk 转成可追踪 requirement，并先建立不会假绿、不会拿错
compiler/runtime、能复现精确候选的 Live 证据系统。本阶段不实现 bytecode/runtime 语义。

## 2. 入口条件

- scope commit manifest 已冻结到 `430e5ff2`；后续设计 amendment 尚未加入 scope。
- 三仓 main checkout 保持 main；实施 worktree 直接创建在 `/Users/geek/workspace/`。
- 当前 chat、host、Actor、tail-call、lazy deployment 证据可被只读审计。

## 3. 交付物

1. Requirement ledger：每个 included semantic hunk 有稳定ID、owner、状态、测试/删除证据和目标阶段。
2. `router-live:agine` managed selector：一个隔离栈顺序运行 chat-smoke、host-tools check和strict full host-tools。
3. Strict host-tools：error/stopped、空答案、零tool call、错误runtime PID和profiling sample缺失都会非零退出。
4. Provenance manifest：三仓commit/tree、compiler/router/runtime路径与SHA、ISA/schema、artifact/buildId。
5. Baseline benchmark manifest：固定机器、release/debug profile、workload、采样窗口、统计口径和baseline commit。
6. Actor exact-build fence、owner-lease/idle-TTL ordering等已知问题的失败测试或明确ledger disposition。

## 4. 非目标

- 不引入bytecode schema或VM类型。
- 不以更新architecture/reference代替requirement accounting。
- 不修复本阶段发现的VM语义缺口；它们进入明确后续阶段。

## 5. 验收

### 5.1 Focused gate

```bash
node scripts/verify.mjs --only tooling
node scripts/verify.mjs --only checks
pnpm --dir scripts type-check
git diff --check
```

若修改 `internals/agine` host/client gate，还必须在该仓库运行相应 type-check/test。Live registry的schema、catalog、
selector graph与测试必须同步通过，`router-live:agine --list`只能展开一个canonical managed invocation。

### 5.2 阶段专属证明

- 对host-tools注入terminal error、stopped、空assistant、零tool call、错误PID、缺sample，逐项证明非零退出。
- 同一候选重复生成manifest，稳定字段一致；修改compiler binary后manifest/SHA必须变化并使旧证据失效。
- harness只使用临时Mongo、动态端口和显式子进程，不触碰stable 4000–4007或稳定host home。
- baseline结果明确标注tree evaluator，不声称VM证据。

### 5.3 强制 Live

先运行现有chat与人工严格host-tools形成bootstrap baseline；随后必须运行新selector：

```bash
node scripts/verify.mjs --only router-live:agine
```

只有新selector在同一个隔离栈完成chat、host check和full host-tools并产出strict manifest，本阶段才可
`candidate-pass`。合并main后按[共同验收合同](README.md#4-合并-main-后的-stable-closure)完成stable closure。

## 6. 停止条件

- 无法证明实际使用哪个compiler/runtime或采样了哪个PID。
- host-tools需要外部日志正则才能弥补脚本自身不判错。
- 为让baseline通过而跳过chat、full host-tools或真实provider。
- requirement ledger把未确认的实现标为complete，或把语义hunk无owner地推到Phase 9。

## 7. Handoff

Phase 1只接受Phase 0 `complete`的gate、manifest schema和baseline。后续阶段不得降低strict assertion；如需修改
harness，必须开启新evidence epoch。
