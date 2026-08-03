# Skiff Dispatch Reference（草案）

状态：正式参考文档。权威契约：[`../architecture/durable-task-dispatch.md`](../architecture/durable-task-dispatch.md)；本文只定义该契约的用户可见 grammar、公开类型与 status / cancel 拼写。

## 1. 定位

- `dispatch` 是 Skiff 唯一的 detached-call surface：提交一个与当前 request 生命周期分离的 callable invocation。
- 立即与延迟是同一机制的不同 eligible time；`dispatch` 不暴露 queue / partition / ack / lease / worker。
- `dispatch` 是表达式，求值结果为 `std.task.TaskRef`；单独成行时按 expression statement 处理，返回值可丢弃（fire-and-forget）。
- `dispatch` 是完全保留名：不得用作函数名、局部绑定名、type / alias / interface 名或 import alias。`after` / `at` 仅在 dispatch 后缀位置保留。
- 提交成功即返回 TaskRef；target 的结果不会返回给调用方，不能 await。

## 2. Grammar

```text
DispatchExpr := "dispatch" CallExpr [ TimingClause ]
TimingClause := "after" "(" DurationExpr ")" | "at" "(" InstantExpr ")"
```

- `CallExpr` 为普通函数调用或 actor 方法调用（见 §4）。
- 无 TimingClause：立即任务，`due_at` = 提交时刻。
- `after(d)`：`d` 为非负 Duration；`due_at` = 求值 timing 时取得的平台 UTC 时间 + d。zero duration 等价立即；负数、overflow、不可表示值拒绝。
- `at(t)`：`t` 为 Instant 值，规范化为 UTC timestamp；`due_at` = t。
- 求值顺序：receiver / 参数各求值一次 → timing 表达式求值一次 → recoverable encode → durable commit。`after` 的基准时间是 timing 表达式求值时取得的平台 UTC 时间。
- dispatch 可出现在任意表达式位置（赋值、参数、if / match 分支等）；单独成行是 expression statement。

## 3. 类型与公开 surface

### std.task.TaskRef

- opaque recoverable value，可跨 request 保存 / 恢复（可进入 DB stored field、persistent payload 等 recoverable 上下文）。
- 内部只承载 TaskId + owner scope；不包含 Runtime 地址、queue partition、lease id 或 mutable task snapshot。
- 不能跨 owner service 操作其他 service 的 task。
- 公开构造只有 `dispatch` 表达式；不提供从裸 TaskId 构造的路径。

### std.task.status

```text
std.task.status(ref: std.task.TaskRef) -> TaskStatus
```

`TaskStatus` 为 discriminated union：

- `{ kind: "scheduled" }`：未来 `due_at`，未到期
- `{ kind: "ready" }`：已到期可调度，未 claim
- `{ kind: "running" }`：已 leased，attempt 执行中
- `{ kind: "succeeded" }`：target 明确返回
- `{ kind: "failed" }`：target 明确 throw / reject
- `{ kind: "platformFailed" }`：永久平台错误
- `{ kind: "canceled" }`
- `{ kind: "expired" }`：retention 已过期 / TaskId 不可解析（稳定结果）

### std.task.cancel

```text
std.task.cancel(ref: std.task.TaskRef) -> TaskCancelResult
```

- `{ kind: "canceled" }`：scheduled / ready → terminal canceled；成功返回后保证不会产生 attempt
- `{ kind: "alreadyStarted" }`：claim 已先成功（leased）；不修改状态、不发 stop hint
- `{ kind: "alreadyTerminal" }`：已 terminal（succeeded / failed / platformFailed / canceled）
- `{ kind: "expired" }`：retention 已过期 / 不可解析

第一版只有 before-start cancellation；没有 running cooperative cancel。

## 4. Target 规则

- target 必须是当前 service 构建内的普通 function（本 service module 或依赖 package 的 function），或 actor 方法（`actor.method(...)` / `self.method(...)`）。
- 跨 service callable 不能作为 target。
- target 返回类型必须 void / null。
- 参数必须满足 recoverable boundary（[`recoverable-value.md`](../architecture/recoverable-value.md) 的 owner-internal envelope）；不可恢复参数在提交前 fail closed，不产生半截 task。
- db transaction 内禁止 dispatch（静态检查；业务一致性由 outbox / reconciliation 处理，不由本机制隐式保证）。
- actor `create` 内禁止 `dispatch self.method(...)`。
- actor-method target 提交时冻结 `ActorActivationSnapshot`；执行走 get-or-activate；旧 implementation 被升级接管时按 platform-failed 拒绝（细节见权威文档）。

## 5. 执行语义（摘要，细节以权威文档为准）

- 提交顺序：求值 → 冻结 execution image / target → recoverable encode → 生成 TaskId → TaskStore durable create（TaskId-idempotent）→ 返回 TaskRef。
- 成功返回保证 task 已 durable commit；响应不确定时内部复用原 TaskId 查询 / 重试。
- 位置透明：不保证提交方 Runtime 执行；立即 task 也没有易失 fast path 语义线。
- at-least-once attempt：lease loss / 不确定 settlement 自动产生后续 attempt（平台 bounded backoff）；普通 request deadline / timeout 是已开始执行，收敛为 `failed`，不自动重跑。
- 同一 task 同时最多一个有效 lease；TaskId 吸收重复 notification；没有业务 dedupe。
- execution image 冻结的是可重新激活的旧 image + artifact retention root，不 pin 旧 Runtime 进程。
- 取消与 claim 竞争同一个 CAS；cancel 先成功则永不执行，claim 先成功则 AlreadyStarted。

## 6. 术语边界

- `dispatch`：本文定义的语言表达式（用户语义）。
- task dispatch / `task.*` wire / TaskStore：持久调度机制（内部）。
- request dispatch（Router `RequestDispatcher` / `DispatchSubmit` / `DispatchMode`）：把普通 request frame 分发给 runtime 的 transport 机制；task attempt 复用该通道执行，但 request dispatch 不拥有 task 语义。
- 文档用语：描述 router 动作时用“派发 / 转发请求”，不用“dispatch”。

## 7. 随本草案一并收敛的文档改动

- `doc/reference/syntax.md` §6：expression statement 枚举增加 dispatch 表达式；保留名规则增加 `dispatch`。
- `doc/reference/static-semantics.md`：`TaskRef` 加入可恢复类型；保留名规则增加 `dispatch`。
- `doc/reference/runtime.md`：移除旧 `spawn` 用户面表述并按本文语义收敛（pending 容量只计 leased attempt 对应的 active request；scheduled / ready backlog 不计入任何 Runtime connection）。v1 不支持 `concurrent`，`dispatch` 与普通能力调用同等对待；concurrent 恢复时再收敛其受限列表。
- `doc/reference/static-semantics.md`：`dispatch` 明确列入保留名（§9 已声明关键字不能作为用户标识符）；§18 recoverable boundary 增加 `std.task.TaskRef` 为可恢复类型。
- `doc/reference/queue.md`：与权威文档的矛盾（自动重试 / lease expiry / build pinning / dedupe）按权威文档收敛；业务源码不暴露 queue。
- `doc/reference/spawn.md` 不再存在；历史表述只保留在权威文档中。

## 8. 测试矩阵

### 语法 / 编译层（parser + compiler）

1. `dispatch foo()` 正例：立即、表达式位置（赋值 / 参数 / 分支）、单独成行（丢弃 TaskRef）。
2. `dispatch foo() after(200ms)` / `dispatch foo() at(t)` 正例；zero duration 正例。
3. 负例：`dispatch` 用作函数名 / 局部绑定名 / 类型名；target 不是 call；`after(-1ms)`；`after` / `at` 参数缺失或类型错误；db transaction 内；actor `create` 内 self dispatch；target 非 void；参数含 callback / Stream / transaction 等不可恢复值。
4. 副作用计数：一次 source operation 中 receiver / 参数 / timing 各只求值一次；TaskRef 正例：可存入 DB stored field、跨 request 恢复后供 `std.task.status` / `std.task.cancel` 使用。

### TaskStore / scheduler 层（unit + integration）

5. create TaskId-idempotent：同 TaskId 重试不产生第二条；同 TaskId 不同 canonical record 冲突拒绝。
6. claim CAS：仅 ready + `due_at <= now` + image 可激活；claim 原子写入 state / lease / attempt generation。
7. lease expiry 竞争：settlement 与 recovery 在 authority time 上 CAS，最多一个成功；旧 lease settlement 被拒。
8. renew / heartbeat 携带 lease id；stale settlement 拒绝。
9. duplicate notification：同 attempt 不产生第二个有效 lease；terminal 后 notification 只清理不重开；cancel 后不恢复 ready。
10. cancel / claim 竞争双向。
11. terminal settlement 幂等：同 lease 相同 terminal 重复写接受；冲突 outcome 拒绝。
12. due scanner：未来 task 到期前不可见；到期后可见；wall-clock 回拨不提前。
13. 状态机全 transition + 非法 transition 拒绝。
14. 错误分类：永久错误 → platform-failed（不 hot retry）；暂时性错误 → bounded backoff 后续 attempt。

### Runtime / execution 层

15. payload decode 使用 linked expected plan；schema 不匹配 fail closed。
16. 新 attempt 使用 fresh request heap / context，不继承 caller 状态；trace 继承 TaskId 并新建 attempt / request span。
17. function target 作为独立 service request 执行。
18. actor-method target 走 get-or-activate 五个分支：live 同 implementation / registry 存在 / registry 丢失用 snapshot 恢复 / 升级 forward target / 旧 implementation 被接管 → platform-failed。
19. 普通 request timeout → `failed`（不是 lease loss，不重跑）。

### 端到端（真实链路）

20. 立即 dispatch：source → compiler → artifact → runtime → router → TaskStore → scheduler → runtime 执行成功。
21. 延迟 dispatch：到期前不可执行；到期后执行；到期前取消成功。
22. 崩溃恢复：runtime 断连 / lease 过期后同 TaskId 新 attempt 执行；重复 effect 允许（at-least-once）。
23. router 重启：已接受 task 不丢；actor registry entry 不丢（Rust router + Mongo durable activation）。
24. 取消：scheduled / ready → canceled 后不执行；leased → alreadyStarted。
25. 执行 image 冻结：发布后旧 task 仍执行提交时冻结的 build；不 fallback latest。
26. retention：terminal 原子释放 artifact root；过期后 status / cancel 返回 expired，不重建 task。
