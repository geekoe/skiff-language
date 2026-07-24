# P5-F180D：Actor 方法调用传输与错误合同结果

状态：Completed

## 直接父任务

- `P5-F180B-actor-method-identity-checkpoint-result.md`

## 结果

已建立独立于普通 service request 的 Actor 方法帧族：

- `actor.method.invoke` 携带 logical Actor ref、必填 epoch、声明 owner、Actor ABI、
  requested implementation、method identity、参数编码版本、deadline 和取消关联号。
- `actor.method.return` 用 invocation id 精确关联返回值和返回编码版本。
- `actor.method.error` 保持
  `ActorUpgradingError`、`ActorVersionRejectedError` 和
  `ActorIncarnationReplacedError` 的独立类型及 Actor/版本/epoch 上下文。
- `actor.method.cancel` 独立表达主动取消和 deadline 超时，不降级成普通 service transport error。

Rust 与 TypeScript 使用同一份严格对等语料。两侧都拒绝缺字段、额外字段、未知 frame/schema
版本、错误 identity 长度或编码、非法 epoch、错误 payload 形态和截断二进制帧。Actor owner
使用 `UnitAddr + FileAddr + actorSymbol` 的等价 wire 形态，调用帧不包含声明副本、方法表或
`ExecutableAddr`。

capability-context 已加入 Actor 调用身份、deadline、取消、typed outcome/error 的语义模型。
Runtime host 只提供专用 Actor handoff，并明确返回 `DispatcherNotImplemented`；它不会把调用转入
普通 request handler 或执行普通 executable。本任务未实现 Router admission、owner 状态机、
Actor 实例存储或方法执行器。

## 验证

- `cargo test -p skiff-runtime-transport --lib`：74/74 PASS
- `cargo test -p skiff-runtime-capability-context --lib`：27/27 PASS
- `cargo test -p skiff-runtime-host --lib`：248/248 PASS
- `cargo check --workspace`：PASS
- `pnpm type-check`（Router）：PASS
- `pnpm vitest run tests/actorMethodProtocol.test.ts`：12/12 PASS
- Router 全量测试：527/534 PASS；7 个失败来自既有 compiler fixture/source 识别和 spawn queue
  时间夹具，与本任务 Actor protocol 改动无关；本任务聚焦测试和 typecheck 全部通过。
- `git diff --check`：PASS
