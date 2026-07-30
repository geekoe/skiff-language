# P5-F180D：Actor 方法调用传输与错误合同

状态：Ready

## 直接父任务

- `P5-F180B-actor-method-identity-checkpoint-result.md`

## 目标

建立 Router 与 Runtime 共同使用的 Actor 方法调用传输合同。合同必须直接承载 F180B 已冻结的
Actor 声明 owner、ABI、implementation 和 method identity，不得把 Actor 调用伪装成普通 service
request，也不得恢复已经移除的 `request.start.actorCall` 元数据。

## 范围

- `runtime/capability-context`
- `runtime/transport`
- `runtime/host`
- `router/src/protocol`
- 上述模块的 Rust/TypeScript 对等测试与拒绝测试

本任务不得实现 Router admission/owner 状态机、Runtime Actor 实例存储或 Actor 方法执行器。

## 必须实现

### 请求

定义专用 Actor method invocation frame，至少精确包含：

- Actor logical key/ref；
- expected epoch；
- Actor declaration owner；
- Actor ABI identity；
- requested implementation identity；
- method identity；
- 参数 payload；
- deadline；
- cancellation correlation。

所有身份字段必须使用现有 canonical identity 类型与编码；不得复制 Actor 声明、方法表或使用
`ExecutableAddr`。缺字段、多余字段、未知版本、错误长度或错误 identity 编码必须失败关闭。

### 返回、错误与取消

- 定义与 invocation correlation 一一对应的 typed return frame；
- 定义 typed error frame，并落实：
  - `ActorUpgradingError`
  - `ActorVersionRejectedError`
  - `ActorIncarnationReplacedError`
- 定义调用取消 frame 和 deadline 的 wire 语义；
- 取消、超时和 Actor 专用错误不得降级为普通 service transport error；
- Rust 与 TypeScript 必须对合法和非法 frame 有严格相同的接受结果。

### 边界约束

- 普通 service request/response 线路行为保持不变；
- Actor method frame 只能进入后续专用 dispatcher，不得被普通 request handler 接受；
- 本任务只允许建立 protocol/control handoff；收到调用后若尚无 dispatcher/executor，应明确报告
  “尚未实现”，不得偷偷执行普通 executable。

## 验证

- Rust/TypeScript strict parity corpus 覆盖 request、return、typed error、cancel、deadline；
- round-trip 覆盖 logical key/ref、epoch、owner、ABI、implementation、method 和 payload；
- 缺 epoch、owner、ABI、implementation、method、correlation 或 deadline 表示错误全部拒绝；
- 错误 identity、未知 kind/version、额外字段、截断 payload 全部拒绝；
- 三类 Actor 专用错误保持精确类型和必要上下文；
- 普通 service transport 回归测试；
- capability-context、transport、host 和 Router protocol 聚焦测试；
- `cargo check --workspace`；
- Router TypeScript typecheck/相关测试；
- `git diff --check`；
- 独立提交并写 `P5-F180D-actor-method-wire-error-contract-result.md`。

