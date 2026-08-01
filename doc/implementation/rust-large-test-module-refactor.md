# Rust 大型测试模块职责重构

日期：2026-08-01

状态：approved for implementation

基线：commit `2b584edc6c30cec4acf8fd2c9e3bde85790fe234`，tree
`d6e2bb3f62d48e10810b688791833cb22d189354`。

本文是本次重构的唯一权威设计。阶段、分支、写集和证据 owner 见
[`rust-large-test-module-refactor-stage.md`](./rust-large-test-module-refactor-stage.md)；阶段文档不得
重定义这里的模块边界或完成标准。

## 2026-08-01 用户授权修订

用户明确授权为满足第 4 节第 4 项的既有 workspace rustfmt gate，仅对现有 test-only 文件
`runtime/linker/src/assembly/tests/cross_package_actor.rs` 做一次机械 rustfmt。该文件是本设计原写集之外的
唯一例外写集；diff 只允许 rustfmt 产生的格式变化，不得修改测试逻辑、函数、属性或任何 linker 行为。

本修订只窄化覆盖第 1 节写集和第 4 节第 6 项中的“无关格式化”限制，不扩张任何其它 production、测试或
格式化范围，也不授权顺手格式化其它文件。除此之外，本文的目标、职责、非目标和完成标准全部不变。

## 1. 目标与边界

基线上的 `compiler/source/src/callable_effects/tests.rs` 为 4849 行、86 个测试，
`runtime/service-db/src/tests.rs` 为 4211 行、102 个测试。两者都把多个已经可命名的测试领域、共享
fixture 和断言集中在一个根模块；后者另有 `tests/prepared_runtime/`，说明子模块布局已经是仓库现有
做法。

本次实现代码只允许修改：

- `compiler/source/src/callable_effects/tests.rs` 及新建的 `callable_effects/tests/**`；
- `runtime/service-db/src/tests.rs` 及既有或新建的 `service-db/src/tests/**`；
- 两条开发线合流并通过检查后，`scripts/check-rust-file-lines.mjs` 的真实最大行数。

此外只可新增阶段文档规定的 leaf/result 证据文档；证据文档不扩大代码写集。

重构只改变测试组织和测试 support。不得改变生产语义、生产 API/ABI、artifact/wire/schema、错误
payload、Mongo 行为或默认/live 测试边界；不得新增 crate、依赖、测试框架、配置或兼容层。复用 Rust
子模块、现有 fixture 和标准库同步原语即可。

## 2. 终态职责

### 2.1 Callable effects

`callable_effects/tests.rs` 终态只声明子模块。领域测试按下列 owner 收敛：

| 子模块 | 唯一职责 |
| --- | --- |
| `analysis_resolution.rs` | pending/default、局部/跨文件 call graph、target resolution、interface/actor receiver resolution |
| `heap_provenance.rs` | fresh/alias、field/container projection、store、cycle/SCC 与返回 provenance |
| `escape_boundaries.rs` | throw/rethrow、stream/spawn/callback、DB write/transaction 等 escape lane |
| `native_functions.rs` | context-free native、HTTP/file/config 与 package native 的精确 summary |
| `receiver_builtins.rs` | Date/String/Bytes/JsonObject/Map 等 receiver builtin 的 contextual transfer |
| `dependencies_contracts.rs` | dependency artifact/signature/field callable 与 contract descriptor/fail-closed |
| `support.rs` | 编译/分析 harness、dependency fixtures、效果与 provenance 的窄断言；不得拥有测试 |

`support.rs` 引入一个 `AnalysisFixture` builder，统一 platform/prelude、source 名称、module/package、
dependency analysis/artifact 等可选输入，消除多套分析入口和 `too_many_arguments`。builder 只表达 harness
配置；每个 Skiff 源码块和真实回归形状仍在对应测试中完整可读，不把行为样例改造成字符串模板或大型
参数矩阵。只抽取具有领域名称且多次使用的窄断言。

### 2.2 Service DB

`service-db/src/tests.rs` 终态只声明子模块。既有 `prepared_runtime` 及其目录保持独立，其他测试归属：

| 子模块 | 唯一职责 |
| --- | --- |
| `error_contract.rs` | wire payload、catch identity、sanitization、Mongo error 分类 |
| `provider.rs` | capability context、provider config/build/provision |
| `runtime_config.rs` | publication/storage identity、database/client cache 与 client options |
| `metadata.rs` | metadata validation、collection/index/lease metadata plan |
| `mapping.rs` | key/query/document、file record、Date 与普通 BSON/RuntimeValue mapping |
| `lease.rs` | lease acquire/renew/lost 行为 |
| `recoverable.rs` | recoverable envelope、interface restore、retention 与 production-context 行为 |
| `mongo.rs` | 不需要真实服务的 conflict/retry/transaction Mongo 行为 |
| `encrypted_mapping.rs` | JSON 与 RuntimeValue 两条 encrypted mapping 路径及伪造 metadata 拒绝 |
| `live_mongo.rs` | 唯一需要本机 MongoDB replica set 的 ignored roundtrip |
| `support.rs` | 通用 metadata/provider/storage/Mongo error fixture；不得拥有测试 |
| `recoverable_support.rs` | recoverable expected plan、heap/interface、artifact/root store 与 hooks fixture；不得拥有测试 |

共享 helper 必须由最窄的 support 文件唯一拥有，测试模块通过 `pub(super)` 等测试内可见性使用，不得为
测试提高生产 API 可见性。`TestDbBehaviorHooks` 与 `ThreadSafeTestDbBehaviorHooks` 收敛为一个线程安全
fixture 和显式计数访问器，删除转发同一 trait 的双实现。重复的常用 Thread binding/metadata 构造可抽为
具名 helper；错误 payload、metadata JSON、加密输入及逐字段断言继续显式，不能用万能 builder 隐藏行为。

## 3. 测试身份与 live 边界

模块拆分必然在 Rust 全名中插入领域模块段，但测试函数名、属性和行为不得改变。每条开发线在移动前后
保存 `cargo test ... -- --list` 输出，并提交到 result 文档的证据摘要；验收按“相同函数名集合、相同数量、
每个旧全名恰好映射到一个新领域全名”做双射检查。不得通过复制、删除、合并或重命名测试来消除差异。

基线 live 测试
`tests::service_db_runtime_create_and_find_runtime_roundtrips_local_interface` 必须唯一移动为
`tests::live_mongo::service_db_runtime_create_and_find_runtime_roundtrips_local_interface`，保留原函数名、
`#[tokio::test]` 和
`#[ignore = "requires a local MongoDB replica set and real network resources"]`。默认验证只编译并列出它，
不得执行它；只有明确选择既有 live 流程时才可运行。仓库中不得出现第二个同名或第二个等价 live 测试。

## 4. 完成标准

同时满足以下条件才算实现完成：

1. 两个根 `tests.rs` 只做模块声明；所有测试和 helper 按第 2 节有唯一 owner，既有
   `prepared_runtime` 行为不变。
2. Callable effects 的分析入口由 `AnalysisFixture` 统一，不再保留被其替代的多入口/过长参数 helper；
   Service DB 的 recoverable fixture 下沉且 hooks 只有一个 trait 实现。
3. 86 个 callable-effects 测试和 102 个 service-db 根测试均按第 3 节双射保留；既有
   `tests/prepared_runtime/**` 测试集合也不增不减。所有 `#[ignore]` 状态不变。
4. focused test、对应 compiler/runtime selector、rustfmt、Clippy/rust-quality 与静态 checks 均通过；
   不以启动独立 runtime/router/Mongo 代替 hermetic 验证。
5. 集成树中所有 `.rs` 文件重新计数后，`MAX_FILE_LINES` 精确更新为当时真实最大值，注释与输出一致，
   仍保持无 allowlist/exception；两个被重构领域文件不得成为新的最大文件。
6. 除 2026-08-01 用户授权修订中的单文件机械 rustfmt 例外外，diff 不含生产文件、manifest/lockfile、schema、
   配置或无关格式化；没有新增依赖、框架或兼容代码。

## 5. 非目标

- 不修改 callable-effects 或 service-db 的生产实现，即使测试审查暴露可进一步改善的生产设计。
- 不重新设计 prepared-runtime 测试体系，不合并 JSON/RuntimeValue 两条行为路径。
- 不追求最少文件或最少测试代码；清晰职责和可审阅 fixture 优先于参数化压行。
- 不把 line gate 设成预估值或人为整数阈值，也不为现有大文件新增例外。

## 6. 验证层级与证据

| 层级 | 必须证据 | owner |
| --- | --- | --- |
| 叶子预检 | baseline commit/tree、测试 list/count、ignored 属性、工作区洁净写集 | 对应开发节点 |
| 叶子验证 | focused tests、前后测试名双射、`cargo fmt --check`、本 crate Clippy/编译结果 | 对应开发节点 |
| 集成检查点 | 两提交可无冲突合流；联合 focused tests；最终测试名/ignore 审计；diff 写集审计 | 集成节点 |
| line gate | 全仓 Rust 行数降序清单、真实最大值来源、更新后 checker 输出 | line-gate 节点 |
| 稳定候选 | 固定候选 commit/tree，禁止后续写入 | 协调父节点 |
| 独立验收 | 对本设计第 1–5 节逐条只读审查，核对测试双射与 live 规则 | 独立 acceptance 节点 |
| 独立 gate | 在固定候选上运行 compiler、runtime、rust-quality、checks 选择器并报告完整命令/退出码 | 独立 gate 节点 |

推荐的聚焦入口为：

```bash
cargo test -p skiff-compiler-source callable_effects::tests
cargo test -p skiff-runtime-service-db --lib
cargo fmt --all -- --check
node scripts/check-rust-file-lines.mjs
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only rust-quality
node scripts/verify.mjs --only checks
```

若失败源自 baseline 已存在问题，证据必须在同一 baseline 命令上复现并明确区分；不得静默跳过或扩大
写集修复无关问题。
