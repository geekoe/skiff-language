# P5-F338 Response error declarative schema fix

状态：Completed。结果见
`P5-F338-response-error-declarative-schema-fix-result.md`。

## 直接父节点

- 独立验收及唯一 blocker：
  `P5-F337-service-error-wire-checkpoint-acceptance-result.md`
- 被验收的 shared checkpoint 结果：
  `P5-F336-service-error-wire-telemetry-checkpoint-result.md`

父节点已沿引用链连接唯一权威设计。本任务只修复 F337 B1，不重新设计 service error wire，也不承接
request/host、Router dispatcher/gateway或 telemetry storage/query consumer。

## 精确候选与目标

- 起点 commit：`ce8035d2c83961effb5d5b01b2825a8dd80262f9`
- 起点 tree：`74ee30f5df209aa94c635b1a6a9fae7d09d471f0`
- 当前 TypeScript interface、manual validator和 `validateResponseErrorFrame(header, payloadBytes)`
  已经表达正确的 exact union；不得改变其 wire 行为：
  - `fixedService` header 禁止 `error`，payload 必须非空；
  - `control` header 必须有 generic `error`，payload 必须为空。
- 唯一 production 缺口是
  `router/src/protocol/runtimeProtocol.ts` 导出的 `runtimeFrameHeaderSchemas['response.error']`
  仍是一个把 `error` 视为可选的 property bag。

完成后，声明式 schema 自身必须表达同一个判别 union：

1. fixedService 分支只允许
   `schemaVersion/type/requestId/errorKind`，禁止 generic `error`和所有额外字段；
2. control 分支要求
   `schemaVersion/type/requestId/errorKind/error`，禁止额外字段；nested `error`仍只允许
   `code/message/status/details`并要求 code/message；
3. 两个分支都保持 response-error v2、固定 type、非空字符串等现有语义；不要新增 v1、fallback、
   兼容 reader或按 code/message升级 fixed；
4. 其它 frame schema 的结构和含义保持不变。

可以为声明式 schema 类型增加表达 exact `oneOf` 所需的最小能力，也可以采用同等严格且不重复
wire owner的表示；不得只在测试里写一个与 production schema 无关的特殊判断。

## 写入边界

唯一允许 production 写入：

- `router/src/protocol/runtimeProtocol.ts`

唯一允许测试写入：

- `router/tests/protocol.test.ts`

不得修改：

- `router/src/protocol/envelope.ts`；
- shared corpus
  `runtime/transport/testdata/service-error-response-v2.json`（它已经包含所需正负例）；
- Rust、telemetry、request/host、Router dispatcher/gateway；
- 权威设计、父任务/result、其他 task；
- package manager/lockfile。

若实现确实需要超出上述边界，先停止并返回 blocker，不要自行扩张。

## 必须实现并证明

1. 声明式 schema 不再是可同时容纳两种 variant 的单一 optional-property bag。
2. 使用同一个 `service-error-response-v2.json`：
   - 所有适用于 header 的合法 fixed/control case 被声明式 schema 接受；
   - 所有 header 非法 case被声明式 schema拒绝，至少显式覆盖
     `fixed-carries-generic-error`和`control-missing-error`；
   - payload-only 非法 case仍由既有 header+payload seam拒绝，不要求 header schema伪装校验 payload。
3. 测试必须真正按声明式 schema求值，不能只断言对象字段长什么样；可以添加最小、通用的 test-side
   evaluator来解释 production schema，或复用 repo中已有 evaluator。
4. 既有 manual/interface/header+payload corpus测试继续通过，payload bytes identity不变。
5. `runtimeFrameHeaderSchemas`的其它调用点仍可类型检查；不得以 `any`、宽泛 cast或跳过 schema分支掩盖。

## 验证

先列出非零 selector，再至少运行：

```bash
pnpm --filter @skiff/router exec vitest run tests/protocol.test.ts
```

再运行 Router 的最小 TypeScript type-check/build命令；若被父 checkpoint故意留下的 H/R/T consumer
断点阻塞，必须给出精确非本任务报错，并用只覆盖本任务文件的编译证据补足，不能声称完整通过。

同时执行：

```bash
git diff --check
```

不得运行完整 workspace/root/stable/live，不 push。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f338-response-error-schema`
- branch：`codex/p5-f338-response-error-schema`
- 新的一次性开发 Agent；
- 提交 production/tests，并新增
  `P5-F338-response-error-declarative-schema-fix-result.md`，写明 exact diff、测试 selector/数量、
  shared corpus如何区分 header 与 payload invalid case、剩余 blocker；
- 返回 implementation commit。主 Agent 合流后运行同一 corpus 的组合探针，再进入精确 blocker复验。
