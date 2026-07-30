# P5-F180L：Actor Compiler→Router→双 Runtime 全链验收

状态：Ready

## 直接父任务

- `P5-F180I-actor-upgrade-drain-transition-result.md`
- `P5-F180J-actor-runtime-crash-cleanup-result.md`
- `P5-F180K-actor-owner-lease-idle-ttl-result.md`

## 目标

闭合 Actor 生产链路，不再停留在 Router transport seam 或 RuntimeHost seam：真实 Skiff 源码生成
Actor artifact；调用通过 Router 专用 Actor protocol 路由到 owner Runtime；两个 Runtime 目标共同验证
并发、挂起、升级、崩溃和逐出恢复。

## 范围

- compiler/artifact/linked Actor method 真实 fixture；
- Router runtime endpoint 与 Actor dispatcher/owner transport 生产接线；
- Runtime transport 接收、RuntimeHost admission/executor、typed return/error/cancel 生产接线；
- upgrade、disconnect、lease/idle 控制帧的生产接线；
- 不依赖 stable instance 的双 Runtime 全链测试。

不得使用直接调用 seam 的假全链，不得改写 artifact、伪造 method table、恢复普通 service request
fallback，也不得声称 exactly-once。

## 必须实现

- 真实源码至少声明一个有字段、同步方法和可挂起方法的 Actor，并由 compiler 生成专用
  `ActorMethod`/`ActorDispatch`；
- Router runtime endpoint 只接受 F180D 专用 frame，执行 F180E admission 后把 admitted invocation、
  owner fence 和首次 activation bootstrap 发往 exact owner Runtime；
- Runtime 通过真实 transport 解码，进入 RuntimeHost 专用 Actor handoff 和 F180G/H executor；
- typed return、三类 Actor error、cancel、deadline 原路关联回调用方；
- upgrade mark/discard/activate、disconnect cleanup、lease renewal/expiry、idle eviction 使用真实 control
  transport，不直接调用另一侧内部对象；
- 两个 Runtime 的 session/owner fence 严格隔离；
- 所有帧保持 Rust/TypeScript strict parity，未知/额外/缺失字段失败关闭。

## 验收场景

- 同实例同步代码段不交错；
- `stream.next()` 已缓冲时不让出，真实等待时让出；另一方法修改字段后，恢复方法看到新值；
- `connection.send` 不让出；
- 不同实例在两个 Runtime 上并行；
- 普通 request/后台 task/外部 capability 不能访问 Actor 字段；
- stale epoch resume 返回 `ActorIncarnationReplacedError`；
- same implementation 跨 service version 复用 live incarnation；
- different implementation 关闭 admission、drain、推进 epoch、从原 bootstrap 激活；
- 旧 implementation 返回 `ActorVersionRejectedError`，不会反向降级；
- replace/remove、Runtime crash、lease expiry、idle TTL 均丢弃 live 字段，保留适用的 registry/bootstrap
  语义并按规则重建；
- Router restart 丢失 registry entry，业务可用 `getOrCreate` 重建；
- 调用失败、取消、重试和外部副作用窗口可观测，测试明确没有 exactly-once 保证。

## 验证

- production Router/Runtime Actor transport 聚焦测试；
- 真实 compiler→artifact→Router→两个 Runtime 全链测试全部通过；
- Router 类型检查与相关全量测试；
- Runtime eval/host/transport 聚焦与全量测试；
- `cargo check --workspace`、`git diff --check`；
- 反向搜索确认 Actor 方法无普通 request/ExecutableAddr fallback；
- 独立提交并写 `P5-F180L-actor-full-chain-acceptance-result.md`。

