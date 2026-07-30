# P5-F180L：Actor Compiler→Router→双 Runtime 全链验收结果

状态：Completed

## 直接父任务

- `P5-F180I-actor-upgrade-drain-transition-result.md`
- `P5-F180J-actor-runtime-crash-cleanup-result.md`
- `P5-F180K-actor-owner-lease-idle-ttl-result.md`

## 结果

Actor 已从此前相互分离的 compiler、Router 和 Runtime seam 接入生产传输链：

- 真实源码产生独立 Actor declaration、method implementation 和 `ActorDispatch`；Actor 名义 handle
  保留为独立 `ServiceSymbol`，不会伪造 `TypeAddr`，也不会回退到普通 executable。
- eval 遇到 `ActorDispatch` 后使用专用 Actor capability，按公开签名编码参数并等待专用返回、
  三类 Actor 领域错误、取消或普通传输失败；不经过 `request.start`。
- Router runtime endpoint 接受 `actor.method.*`，从 committed RuntimeAssembly 引用的真实
  PackageArtifact/FileIr 建立 method catalog，完成准入、owner 选择、lease、pending correlation
  和 ledger terminal。
- Router→owner Runtime 使用独立 `actor.owner.invoke`；Runtime 再次校验完整 owner fence，从同一
  committed assembly snapshot 建立执行上下文，物化/跟踪实例并进入 F180G/H executor。
- `actor.owner.control` 生产帧接通 upgrade mark/discard/activate 和 idle eviction；每一步都有
  request/runtime/operation 精确关联的 ACK。Runtime 断连继续使用 F180J 的完整 session fence 清理。
- 未知执行错误使用内部 `actor.owner.failure`，Router 将 ledger 标为 failed，caller Runtime
  收到普通 capability transport error；它不会被伪装成 `Cancelled` 或第四种 Actor 领域错误。
- Rust/TypeScript Actor method 协议统一拒绝非 canonical Base64、超出 JavaScript safe integer
  的数值、未知/额外/缺失字段；Rust encode 与 decode 同样 fail closed。

真实全链验收使用临时 Mongo replica set、真实 Router 和两个真实 Runtime，不连接 stable instance：

1. compiler 从 `.skiff` Actor 源码生成 canonical package/deployment/assembly records；
2. Router 激活 generation 1，两个 Runtime 都完成 prepare/commit；
3. HTTP ingress 在 caller Runtime 执行 `getOrCreate` 和 Actor method dispatch；
4. Router 从健康 assembly replicas 选择精确 owner，发送 admitted invoke；
5. owner Runtime 执行方法并原路返回；
6. 同一 logical Actor 连续返回 `actor-count-1`、`actor-count-next`，证明 live 字段跨调用保留。

验收过程中还闭合了真实链才暴露的生产缺口：

- RuntimeAssembly 数组排序原先依赖 Rust struct 的对象字段插入顺序，落盘后无法被 TypeScript
  重现；现在 Rust 和 TypeScript 都以 canonical JSON bytes 排序。
- isolated test runtime 现在初始化单节点 Mongo replica set、等待 PRIMARY，并写入 generation 0
  activation state。
- `std.actor.getOrCreate/replace/find/remove` 补齐精确 native callable semantics。
- Actor registry native 调用使用 linked declaration 的 Actor implementation identity，不再错误
  使用整个 service build identity。
- Actor registry native plan 对 T0 使用专用 Actor handle 语义，不尝试把它解析成普通 type
  descriptor。
- candidate activation snapshot 保留真实 Actor method catalog；owner 候选来自当前 assembly 的
  健康、匹配 service deployment binding 的 replicas。

## 验证

- `node scripts/run-actor-full-chain-acceptance.mjs`：PASS
  - 两个不同真实 Runtime replica；
  - generation 1；
  - 结果 `["actor-count-1", "actor-count-next"]`。
- `cargo test -p skiff-compiler --test actor_dispatch_linking`：1/1 PASS。
- `cargo test -p skiff-runtime-eval actor_`：36/36 PASS。
- `cargo test -p skiff-runtime-eval --lib`：118/118 PASS。
- `cargo test -p skiff-runtime-host actor_`：13/13 PASS。
- `cargo test -p skiff-runtime-host --lib`：260/260 PASS。
- `cargo test -p skiff-runtime-transport actor_`：10/10 PASS。
- `cargo test -p skiff-runtime-transport --lib`：77/77 PASS。
- Router Actor/catalog/production WebSocket 聚焦测试：11/11 PASS。
- Router 除既有阻断文件外：45 files、460/460 PASS。
- Router type-check：PASS。
- `cargo check --workspace`：PASS。
- `git diff --check`：PASS。
- 反向搜索确认 `ActorDispatch` 不存在普通 request 或 `ExecutableAddr` fallback。

Router 全量测试在最终代码前一轮为 550/558；修复本任务引入的 WebSocket identity 期望后，剩余
失败仅在四个既有阻断文件：五项旧 compiler authoring fixture 报
`unknown authoring object ... router-websocket-fixture`，两项 spawn queue 时间夹具失败。
这些文件之外的 Router 测试已单独全量通过。

真实跨进程验收聚焦生产主链、双 Runtime 和字段持久化。挂起恢复、同步片段互斥、不同实例并行、
升级 drain、旧 epoch/implementation 拒绝、cancel/deadline、断连、lease expiry 和 idle eviction
由同一生产协议入口的 Router/Runtime 聚焦测试覆盖；没有用 seam 测试冒充 compiler→Router→Runtime
主链，也没有声明 exactly-once。
