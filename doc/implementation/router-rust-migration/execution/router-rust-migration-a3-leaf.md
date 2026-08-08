# Router Rust Migration Batch 4 — A3 Leaf Task

日期：2026-08-02
状态：execution leaf（开发 Agent：`/root/dev_a3`；一次性有界会话）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），重点
  §2.4（actor routing projection contract）、§3.2（stateless
  `ActorMethodCatalogView` / actor owners）、§3.3（immutable `RoutingEpoch` 内
  actor index 由 artifact loader 一次构造）、§3.8（bounded blocking store）、
  §5.3（M-artifact）、§7 E-actor-rust（catalog 只读 A0 projection，不读
  PackageArtifact/File IR）。
- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-4.md`（A3 节点条款、
  写边界、验证 owner）。
- A0 契约：`doc/implementation/router-rust-migration/contracts/router-rust-migration-a0-contract.md`（frozen，
  本节点只消费不修改）。
- A0 叶子：`doc/implementation/router-rust-migration/execution/router-rust-migration-a0-leaf.md`。
- C-model-artifact 契约：`doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-artifact-contract.md`
  （strict reader boundary：不得绕过、不得路径猜测、不得复制 DTO/私有兼容层）。
- C-bootstrap 契约：`doc/implementation/router-rust-migration/contracts/router-rust-migration-c-bootstrap-contract.md`
  （`RoutingEpoch` 的 actor projection 字段归 A0，本叶子只引用不定义）。

## Baseline / worktree

- Repo：`/Users/geek/workspace/skiff`；基线 `main@7683b7c8`（`git rev-parse`
  已验证）。
- 分支：`feat/router-rust-a3`；worktree：`/Users/geek/workspace/wt-a3`。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-a3/target`（worktree 内独立 target）。
- 集成 Agent：`/root/router_rust_integration_b4`；不 merge、不 push、不碰集成
  分支。完成时直接交接并通知 root。

## 零 worktree 只读预检证据（main@7683b7c8）

1. TS 违规路径职责（本节点只读，不改）：
   - `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts`：
     `loadActorMethods` 遍历 `RuntimeAssembly.packageLinkPlan.codeSlots[]` →
     `PackageArtifact` 记录 → `files[].fileIrIdentity` → `FileIr` 记录 →
     `actorDeclarations`，用 `abi.actorName`（source symbol）构造
     `RuntimeAssemblyActorMethod { declarationOwner { unit, file, actorSymbol },
     actorAbiIdentity, actorImplementationIdentity, methodIdentity }`。即扫描
     PackageArtifact / File IR / source symbol，违反 §2.4 canonical topology。
   - `router/src/router/runtimeAssemblyActorMethodCatalog.ts`：
     `hasMethod` 按 (abi, implementation, method, declarationOwner) 匹配；
     `declarationOwnerFor` 返回 (abi, implementation) 的唯一 declaration owner。
     消费者：`actorMethodDispatcher.ts`、`productionActorMethodRouter.ts`、
     `actorGetCreateActivationCoordinator.ts`。
2. deployment crate 现有 strict reader：
   - `skiff-deployment::storage::CanonicalArtifactStore`（`io.rs`/`records.rs`）：
     路径推导 → bytes → duplicate-key 拒绝的 strict JSON（`StrictJsonValue`）→
     raw identity 精确相等 → typed Deserialize（`deny_unknown_fields`）→ 内容
     identity 校验 → canonical bytes 校验。内部 helper（`strict_value` /
     `typed_from_value` / `ensure_canonical` / `canonical_bytes`）均为
     `pub(crate)`，Router consumer 不能复用，必须在 `router/src/artifact/` 建立
     自己的严格边界（不复制 DTO，只消费 `skiff-deployment` 的 canonical 类型）。
   - `skiff-artifact-identity` 提供 `ArtifactRelativePath`（escape-proof 解析），
     `skiff-canonical-json` 提供 `canonical_json_bytes`（与 deployment 写入侧同源）。
3. skiff-router（PR 0b 后）结构：`src/config/`、`src/listener.rs`、`src/main.rs`、
   `src/lib.rs`；无 `src/artifact/`。Cargo 生产依赖仅 transport/request-contract
   + hyper/tokio 等；`skiff-deployment` / `skiff-artifact-identity` /
   `skiff-canonical-json` 均已在 workspace Cargo.lock 内，不引入新外部 crate。
4. A0 canonical 类型：`skiff-deployment::projection::actor_routing` 的
   `ActorRoutingProjection` / `ActorRoutingMethod` / `ActorRoutingRef` /
   `ActorRoutingProjectionError` / `ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION`；
   serde `camelCase` + `deny_unknown_fields` + 构造期校验（排序、唯一、
   identity 前缀、serviceId 一致）。A1 producer 尚未合入；投影记录 ref / 记录
   路径的 canonical 推导未在任何契约冻结（A0 §1 把 bootstrap/artifact refs 留给
   contracts-bootstrap；C-bootstrap 只声明 epoch 有 actor projection 字段）。

## 设计决策（A3 授权范围内；不改变 A0/设计语义）

### D1：consumer 只读 A0 canonical 类型，不复制 schema

`router/src/artifact/` 只消费 `skiff-deployment::projection::actor_routing`
的类型与常量。投影 JSON 的 typed 反序列化、构造不变式全部由 deployment 侧
`deny_unknown_fields` + `TryFrom` 完成；router 不定义任何投影 DTO 副本。

### D2：record ref seam（与 A1 合流时对接）

A0 未冻结投影记录的 identity / record path 推导（A1 producer 输出面）。
本叶子按任务指示先按契约 corpus 开发：consumer 侧定义
`ActorRoutingProjectionRef { record_path: ArtifactRelativePath }`，只携带
已校验的相对路径，不猜测路径、不发明投影级 identity。A1 / contracts-bootstrap
合流时把该 ref 换成 canonical 推导（例如由 RuntimeAssembly / committed bootstrap
产出），reader/loader 校验链不变。

### D3：strict reader 校验链（fail closed）

`ActorRoutingProjectionStore::load(reference)`：

1. `ArtifactRelativePath::resolve_existing` 解析（escape-proof、root 包含性）；
2. 有界读取（`MAX_ACTOR_ROUTING_PROJECTION_RECORD_BYTES = 16 MiB`，与
   C-model-artifact snapshot budget 对齐；超限/缺失 fail closed）；
3. duplicate-key 拒绝的 strict JSON 解析；
4. raw `schemaVersion` 与 `ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION` 精确相等；
5. typed Deserialize 为 `ActorRoutingProjection`（`deny_unknown_fields` +
   构造校验：schema version、identity 前缀、serviceId 一致、排序、重复拒绝）；
6. `skiff_canonical_json::canonical_json_bytes(&projection)` 与原始 bytes 完全
   相等（非 canonical fail closed）。

全部失败路径返回 typed `ActorRoutingProjectionError`，不产生 partial 投影。
同步 reader 的 bounded blocking pool / deadline 归 W-bootstrap（C-bootstrap
§2.3）；本模块保证 `Send + Sync`，可直接经 `spawn_blocking` 调用。

### D4：artifact loader 构造 immutable actor index

`ActorRoutingCatalog::from_projection(Arc<ActorRoutingProjection>)` 一次构造
immutable、按完整 typed key 排序去重的索引（投影构造已保证），支持完整 key
精确查询、actor-scoped 遍历与 entry 迭代。查询语义（admission / owner control）
归 C-actor/W-actor；本叶子只交付 epoch-local 的 index 构造证据（§3.3）。
`ActorRoutingProjectionStore::load_catalog` 提供 read + index 的 loader seam。

### D5：共享 corpus 位置与消费

corpus 放 `deployment/tests/fixtures/a3-actor-routing/`（A0 类型 owner 侧，
未来 A1 producer 与 A3 consumer 同读同一 corpus）：

- `corpus.json`：schema `skiff-router-rust-actor-routing-corpus-v1`，case 列表
  （name + 精确 record content bytes + expected outcome）；合法记录（含空投影、
  多 entry、单 entry）与负例记录（schema version、identity 前缀、serviceId 一致、
  duplicate、deny_unknown_fields 反例：`modulePath`/`actorName`/`methodName`/
  `fileIrIdentity`/`codeSlot`/`executableIndex`/`sourceSpan`/`actorSymbol`/
  `actorTypeIdentity`、非 canonical、unsorted（构造归一化后 bytes 不再 canonical）、
  duplicate JSON keys、malformed、missing）都以内嵌 content 字符串给出；
  测试在临时 artifact root 物化 content 后经真实文件边界读取（与
  `bootstrap_artifact_reader_corpus.rs` 的临时 root 模式一致）。

deployment 测试（`a3_actor_routing_corpus.rs`）验证 A0 typed 边界与 corpus 一致；
router 测试（`artifact_actor_routing_corpus.rs`）验证 strict reader/loader 在真实
临时 root 与 fixture root 上的正负例与 canonical 边界。

## 写集

生产（仅 A3）：

- `router/src/artifact/mod.rs`、`router/src/artifact/strict_json.rs`、
  `router/src/artifact/actor_routing.rs`、`router/src/artifact/catalog.rs`；
- `router/src/lib.rs` 只加 `pub mod artifact;`（additive）；
- `router/Cargo.toml`：加 workspace 内已有依赖 `skiff-deployment`、
  `skiff-artifact-identity`、`skiff-canonical-json` 与已 pin 的 `serde` /
  `serde_json` / `thiserror`。

测试 / 文档：

- `router/tests/artifact_actor_routing_corpus.rs`（artifact_* 前缀）；
- `deployment/tests/a3_actor_routing_corpus.rs`（a3_* 前缀）；
- `deployment/tests/fixtures/a3-actor-routing/corpus.json`（内嵌 record content，
  records 文件不落盘）；
- 本叶子文档 `doc/implementation/router-rust-migration/execution/router-rust-migration-a3-leaf.md`。

禁止写：`router/src/session/`、`router/src/activation/`、`router/src/main.rs`、
`router/src/listener.rs`、`runtime/transport`、verify 注册表、AGENTS.md、
scripts README、verify.yml、`skiff-instance.mjs`、A0 契约与 canonical 类型、
`deployment/src/` 生产代码。不操作 stable instance / Mongo / PM2 / 4004-4007；
不跑全量 `pnpm verify`。

## 自验收矩阵（提交前执行）

| 项 | 命令 / 断言 |
| --- | --- |
| router artifact 测试 | `cargo test -p skiff-router --test artifact_actor_routing_corpus --no-fail-fast` |
| deployment 相关测试 | `cargo test -p skiff-deployment --test a3_actor_routing_corpus --no-fail-fast` |
| 全 crate 聚焦回归 | `cargo test -p skiff-router -p skiff-deployment --no-fail-fast` |
| closure 负例 | `cargo tree -p skiff-router -e normal` 不含 `skiff-runtime-model` / `skiff-runtime-host` / `skiff-runtime-eval` / request execution |
| 不读 File IR/source/payload | `rg` 负例：`router/src/artifact/` 与 `router/tests/artifact_*` 不含 `FileIr` / `PackageArtifact`（DTO 类型）/ `modulePath` / `actorName` / `methodName` / `sourceSpan` / `executableIndex` / `payload` 读取路径 |
| rustfmt | 触碰文件 `cargo fmt --check`（聚焦文件） |

## 停止条件

- 发现 A0/设计未覆盖且改变架构或公共契约语义的决策：停止返回
  `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE` 附证据。
- 兄弟 ownership 冲突（router `src/lib.rs` / deployment 生产代码）：先通知 root
  与集成 Agent，不静默改写。

## 执行结果（提交前自验收）

- `cargo test -p skiff-router --test artifact_actor_routing_corpus --no-fail-fast`：
  8 passed（corpus 形态、正例、负例 fail-closed 矩阵、canonical roundtrip、
  catalog index、oversized/missing/escape/root 边界探针）。
- `cargo test -p skiff-deployment --test a3_actor_routing_corpus --no-fail-fast`：
  5 passed（typed 边界与共享 corpus 一致、deny_unknown_fields 反例、
  schema/malformed/非 canonical、构造归一化）。
- `cargo test -p skiff-router -p skiff-deployment --no-fail-fast`：全量通过
  （router 75 unit + 既有 integration suites 全绿）。
- `cargo tree -p skiff-router -e normal` 负例：不含 `skiff-runtime-model` /
  `skiff-runtime-host` / `skiff-runtime-eval` / request execution。
- `rg` 负例：`router/src/artifact/` 与 `router/tests/artifact_actor_routing_corpus.rs`
  无 File IR / source / executable payload 代码引用（仅文档注释中的否定表述；
  File IR 反例只存在于 deployment 侧 corpus 测试数据）。
- `cargo fmt -p skiff-router -p skiff-deployment -- --check`：通过；
  `cargo clippy -p skiff-router -p skiff-deployment --all-targets`：exit 0
  （仅既有 advisory warning）。

## A1 合流对齐点

- `ActorRoutingProjectionRef { record_path }` 是 consumer 侧临时 seam；A1 /
  contracts-bootstrap 提供 canonical 记录身份 / 路径推导后，只替换 ref 构造，
  reader 校验链与 catalog 不变。
- 共享 corpus `deployment/tests/fixtures/a3-actor-routing/corpus.json`
  （schema `skiff-router-rust-actor-routing-corpus-v1`）内嵌精确 record bytes，
  A1 producer 测试可直接消费同一 corpus。

## 交接

完成后直接向 `/root/router_rust_integration_b4` 交接（commit SHA、worktree 路径、
自验收证据、A1 对齐点），并通知 root。
