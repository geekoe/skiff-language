# Router Rust Migration C-net：final listener mechanism contract

日期：2026-08-02
状态：frozen（C-net 交付；供 PR 0b 直接消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md` §5.2（C-net 与 PR 0b）、
  §6.2(2)、§7 PR0b。冲突时以权威设计为准。
- 父批次：`doc/implementation/router-rust-migration-batch-2.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration-c-net-leaf.md`。
- 验证：`router/tests/net_probe.rs`（真实 socket probe）。

本文档冻结 final listener 的**机制**（library、runtime 形态、类型、生命周期与
connection limit 模式），不冻结 HTTP 业务 ports/paths，不定义业务协议或控制端点。
具体 port/path、config 解析与 listener 装配由 PR 0b（C-config 之后）实现。

## 1. Tokio runtime

- 选择：tokio 1.x multi-thread runtime（`rt-multi-thread` + `macros`），
  `#[tokio::main]` 形态启动。workspace Cargo.lock 固定 tokio 1.52.3；
  `runtime/host` 已使用 tokio 1.x 同族 feature（`fs`/`macros`/`net`/
  `rt-multi-thread`/`signal`/`sync`/`time`）。
- 本批次仅在 `router/Cargo.toml` `[dev-dependencies]` 声明 probe 所需 feature
  （`io-util`、`macros`、`net`、`rt-multi-thread`、`sync`、`time`）；PR 0b 把
  production 依赖与 feature 收敛到最终清单。
- 反事实：移除 multi-thread runtime 后，最终 Router 需同时服务 public HTTP、
  runtime/control WS 等多组 listener 与任务，单线程 current-thread runtime 无法
  满足并发生命周期需求；multi-thread 是 workspace 已用且最小的选择。

## 2. HTTP server / upgrade library

- 选择：hyper 1（`hyper::server::conn::http1::Builder`，features `http1` + `server`）
  + `serve_connection(...).with_upgrades()`；hyper-util 0.1 仅用作
  `hyper_util::rt::TokioIo`（tokio ↔ hyper `Read`/`Write` 适配，feature `tokio`）。
- 实测修正（probe 发现）：hyper 原生 `http1::Connection`（不带 `.with_upgrades()`）
  不会完成 upgrade，`hyper::upgrade::on` 会以 `hyper::Error(User(ManualUpgrade))`
  失败；必须使用 `UpgradeableConnection`（`I: Send`，`TokioIo<TcpStream>` 满足）。
  graceful shutdown 使用 `UpgradeableConnection::graceful_shutdown()`（见 §5）。
- hyper 1.9.0 / hyper-util 0.1.20 已在 workspace Cargo.lock（经 `reqwest` 传递引入）；
  本决策不引入任何新版本或新 crate。
- HTTP/1.1 是当前唯一需求：TS Router（Node `http`）是 HTTP/1.1；Runtime 控制 WS
  upgrade 仅支持 HTTP/1.1；设计未要求 HTTP/2。不启用 hyper `http2` 或 hyper-util
  `server-auto`（二者都会把 `h2` 引入 lock），baseline Cargo.lock 也没有 `h2`。
  若未来需要 HTTP/2，属新的独立契约决策。
- WebSocket upgrade 使用 hyper 原生 upgrade 路径：service 检测 `Upgrade: websocket`，
  调用 `hyper::upgrade::on`，返回 `101` 响应（含 `Upgrade`、`Connection: upgrade`、
  `Sec-WebSocket-Accept`），hyper 完成 TCP upgrade 后把 `Upgraded` 交给 WS 层。
- 反事实：删除 hyper/hyper-util 后必须自研 HTTP 解析、keep-alive、chunked、
  upgrade 与 shutdown，超出 PR 0b 最小需求且与 workspace 既有依赖重复；删除
  `TokioIo` 则需手写 tokio ↔ hyper IO 适配，是已存在机制的重实现。

## 3. Body streaming type

- 选择：`http_body::Body` trait，`Data = bytes::Bytes`，错误类型 `hyper::Error`。
  - request body：`hyper::body::Incoming`（流式，支持 backpressure）。
  - service 响应：固定/empty body 用 `http_body_util::Full<Bytes>`；
    boxed 流式边界类型用 `http_body_util::combinators::BoxBody<Bytes, hyper::Error>`
    （PR 0b 用于 streaming 响应）。
- `http_body-util 0.1.3`、`http 1.4.0`、`bytes 1.11.1` 均已在 baseline lock。
- 不引入自定义 body adapter、`BodyDataStream` channel 或 tower/axum 层。
- 反事实：hyper service 必须返回 `Body` 实现；删除 `http-body-util` 需要手写
  `Empty`/`Full`/`Box` 组合器，纯重复实现；删除 `Bytes` 作为 `Data` 会偏离
  hyper/workspace 生态默认（reqwest/hyper 均用 `Bytes`）。

## 4. WebSocket library

- 选择：tokio-tungstenite 0.26（workspace 已由 `runtime/host` 使用，lock 0.26.2）。
- 模式：hyper upgrade 完成后，用 `derive_accept_key`（tungstenite 0.26 公开函数）
  在 101 响应中写入 `Sec-WebSocket-Accept`，然后
  `tokio_tungstenite::WebSocketStream::from_raw_socket(upgraded, Role::Server, None)`
  接管 framing。客户端侧 probe 使用同库 `connect_async`。
- 不在此层做业务参数/JSON-RPC 解析（归 E-ws / C-ws lane）；C-net 只证明
  empty frame roundtrip。
- 被拒绝候选：
  - `tokio_tungstenite::accept_async(hyper::upgrade::Upgraded)`：hyper 已写出
    101 响应后 `accept_async` 会再执行一次 server handshake，造成重复 101；
    因此采用 hyper 101 + `from_raw_socket(Role::Server)`。
  - 纯 `TcpListener` + tungstenite（不经 hyper）：无法与 HTTP listener 统一
    keep-alive/body/shutdown 生命周期，PR 0b 需要单一 listener 服务 HTTP 与 WS。
  - axum / warp / actix-ws：引入整套框架或新生态，workspace 无既有使用，
    且当前需求（upgrade + frame）由 hyper + tokio-tungstenite 最小闭合。
- 反事实：删除 tokio-tungstenite 后无 WS server 能力，Runtime 主动
  `ws://.../runtime` 与 client WS 无法建立，违反设计外部拓扑与 E-ws gate。

## 5. Graceful shutdown

- 选择：三层最小组合：
  1. accept loop 用 `tokio::sync::watch` 信号停止接受新连接；
  2. 每个连接任务 select 同一个 watch 信号，收到后调用 hyper
     `UpgradeableConnection::graceful_shutdown()` 并等待连接自然结束
     （完成当前请求后关闭，空闲连接立即关闭）；
  3. drain 总 deadline 超时后 abort 剩余连接任务（`JoinSet::shutdown`），保证
     shutdown 有界、不悬挂。
- 升级后的 WS 连接脱离 hyper 连接 future（upgrade 完成即结束），必须由
  supervisor 独立跟踪（spawn 到 JoinSet），在 drain deadline 后 abort；本决策
  冻结该跟踪机制。完整的 C-process-lifecycle 停机顺序（stop admission、
  in-flight reconciliation、session barrier 等）由后续 lane 在 supervisor 上展开，
  C-net 只冻结 socket/listener 层的 drain 机制。
- 反事实：删除 accept-stop 后 shutdown 期间仍会接受新连接；删除
  `graceful_shutdown()` 后只能 drop 连接（客户端收到无响应 EOF，非 graceful）；
  删除 deadline/abort 后慢客户端可无限阻塞退出。三者任一缺失都不满足 PR 0b 的
  shutdown 需求，且都是 hyper/tokio 已提供的最小原语。

## 6. Connection limits

- 选择：accept 时 `tokio::sync::Semaphore::try_acquire_owned()`（per-listener
  `Arc<Semaphore>`，容量由 PR 0b 的 config 提供）；超限连接立即写
  `HTTP/1.1 503 Service Unavailable` + `Content-Length: 0` + `Connection: close`
  并关闭；permit 由连接任务持有至连接结束（含 keep-alive 多请求）。
- 语义边界：升级为 WS 后连接脱离 hyper 连接，permit 随之释放；WS 连接自身的
  容量/saturation 归 C-client-lifecycle / C-ws lane 的 socket generation 语义，
  不由 C-net 的 listener cap 覆盖。
- 被拒绝候选：tower-governor/tower limit（引入新依赖与中间件层）、
  hyper-util 内置并发限制（不存在）、每 listener 独立 task 手动计数（Semaphore
  是 tokio 已有最小原语，且支持 `acquire_owned` 跨 task 持有）。
- 反事实：删除 Semaphore cap 后 listener 无并发上限，无法满足设计对
  pre-auth/connection 饱和的 fail-closed 要求（§3.2、§10）以及 PR 0b listener
  的基本资源边界；Semaphore 不引入新依赖。

## 7. 被拒绝候选汇总（简短）

| 候选 | 理由 |
| --- | --- |
| axum / warp / actix-ws | 引入新框架生态，无 workspace 既有使用；hyper + tokio-tungstenite 已最小闭合 |
| `accept_async` on hyper `Upgraded` | hyper 已写 101，二次 handshake 破坏协议 |
| 纯 tungstenite TCP server | 失去 HTTP 层，无法与 HTTP listener 统一生命周期 |
| hyper 原生 `Connection`（无 `with_upgrades`） | probe 实测 upgrade 失败（`ManualUpgrade`），必须用 `UpgradeableConnection` |
| hyper-util `server-auto` / `GracefulShutdown` | `server-auto` 强制引入 HTTP/2（`h2`）；`GracefulShutdown` 不支持 hyper 原生 `UpgradeableConnection`，watch + 原生 `graceful_shutdown()` 更小 |
| tower-governor 等限流中间件 | 新依赖；`Semaphore` 已足够且是 tokio 原语 |
| 手写 HTTP 解析 / 自研连接计数 / 自研 drain | 全部是对 workspace 既有依赖的原语重实现 |

## 8. 本批次验证映射

机制项 | 真实 socket probe | 断言
--- | --- | ---
Tokio multi-thread runtime | 全部 probe 运行在 `#[tokio::test(flavor = "multi_thread")]` runtime | 测试通过
hyper http1 listener | `empty_http_request_response` | `200` + empty body
hyper upgrade + tokio-tungstenite | `empty_websocket_upgrade` | `101` + empty frame echo
Semaphore connection limit | `connection_limit_rejects_overflow_and_releases` | 超限 `503`，释放后可接受
Graceful shutdown drain | `graceful_shutdown_drains_in_flight` | 在飞请求完成后退出，新连接拒绝
Graceful shutdown deadline | `graceful_shutdown_aborts_stragglers` | 悬挂连接在 deadline 后 EOF，server 退出

机制冻结后，PR 0b 按本文档装配 final listeners，不再重新选型。
