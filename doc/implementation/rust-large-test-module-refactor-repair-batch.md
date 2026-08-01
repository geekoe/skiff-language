# Rust 大型测试模块重构：第一次 F/G 失败后的修复批次

日期：2026-08-01

状态：repair DAG ready；R2 等待用户授权

本批次直接承接
[`rust-large-test-module-refactor-stage.md`](./rust-large-test-module-refactor-stage.md) 的 E → F/G 失败路径，事实输入为
[`rust-large-test-module-refactor-acceptance-result.md`](./rust-large-test-module-refactor-acceptance-result.md) 与
[`rust-large-test-module-refactor-gate-result.md`](./rust-large-test-module-refactor-gate-result.md)。唯一权威设计仍是
[`rust-large-test-module-refactor.md`](./rust-large-test-module-refactor.md)；本文只重排执行 DAG、owner 和证据，
不改变设计语义或完成标准。

进入状态为旧稳定候选 E：commit `0a94d75b3d916e87ff2b0c3ea32bebcbda4fc4fe`，tree
`2f98ed10a01a4f6d8b945d34b9af5d885e797d88`。E 已退回预验收候选；不得在 E 上继续实现，也不得继续引用其 F/G
结果声明完成。

## 修复 DAG

```text
R1 Service DB owner 机械修复 ───────────────┐
                                             ├─> batch integration
P1 gate 环境预检与冻结前准备 ──────────────┤       └─> combined probe
                                             │             └─> E2 freeze
R2 无关 linker rustfmt 用户决策 ───────────┘                    ├─> 同一 F 只复验精确 blocker
  └─ blocked；若授权才实施，若不授权则阶段保持 blocked          └─> 新 G 完整 gate
```

R1 与 P1 可以并行。R2 在用户给出明确决定前保持 blocked，不启动实现 Agent。只有 R1 已合流、P1 完成、R2 获得
必要决定且（如授权）对应修复已合流，才能运行 combined probe 并冻结 E2；如果用户不授权但 authority 仍要求
workspace rustfmt 全绿，则不能把该状态冻结为可 release 的 E2。

## R1：Service DB owner 机械修复

状态：ready。

由新的 Service DB 开发 owner 从 E 之后的最新集成 commit/tree 做零-worktree 预检，再创建独立一级 worktree。
唯一源码写集是 `runtime/service-db/src/tests/**` 中以下三项机械移动：

| 函数 | 从 | 到 |
| --- | --- | --- |
| `service_db_capability_context_does_not_require_request_frame` | `error_contract.rs` | `provider.rs` |
| `object_metadata_accepts_retention_field` | `runtime_config.rs` | `metadata.rs` |
| `skiff_file_record_document_preserves_capability_record_fields` | `metadata.rs` | `mapping.rs` |

R1 必须逐字保持每个函数的名称、`#[test]` / `#[tokio::test]` / `#[ignore]` 属性和函数体，维持 102 个原根测试的
双射；只允许为机械移动闭合目标文件已有 import，不改变 helper、fixture、断言、生产语义、公开表面、live 边界或
其它测试 owner。既有 leaf/result 和本批次三份记录均不在 R1 写集；尤其不顺手修 non-blocking 文档。

R1 自验收至少包含三个函数的唯一 owner 搜索、移动前后函数/属性/函数体身份比较、102 测试双射、
`cargo test --package skiff-runtime-service-db --lib -- --list`、crate 范围 rustfmt 与 `git diff --check`。动态 focused
测试是否重跑按失效范围由 R1/集成 owner 决定；不得运行完整 compiler/runtime/rust-quality/checks gate。

## R2：无关 linker baseline rustfmt

状态：blocked，awaiting user authorization。

待决定的问题只有一个：是否授权格式化并提交
`runtime/linker/src/assembly/tests/cross_package_actor.rs`。该文件不属于原阶段写集，且初始 baseline 与 E 都是 blob
`8bbe074b19c47f6178298a5a59950e876baeea86`；baseline 归因明确，但 authority 又要求 workspace rustfmt PASS。

任何 Agent 不得把 F/G finding、旧 task 或实现方便当成扩写授权。若用户授权，主 Agent 必须为 R2 建立最小独立
写入 owner，只接受 rustfmt 的机械 diff、`cargo fmt --all -- --check` 和精确写集证据；不得顺手修改 linker 行为或
其它格式文件。若用户不授权，应如实报告 strict authority blocker，不能将 rust-quality 写成 PASS。

## P1：gate 环境预检与准备

状态：ready；与 R1 并行，但不产生候选 PASS 证据。

P1 先只读确认以下事实并形成可执行准备方案：

- 是否存在可安全复用、与待测精确代码状态一致的 Cargo target；不同代码状态不得并发写同一 target。
- 当前磁盘余量、预计 runtime selector 峰值、明确 target/cache 路径及清理 owner，避免再次在约 6.1 GiB 后
  `ENOSPC`。
- `node_modules` 的可信来源与 `yaml` 的可导入性；detached gate worktree 不得再假设依赖天然存在。
- 四个 selector 的展开命令、工作目录、源码 identity 与缓存隔离方式。

必要的依赖/缓存准备必须在 E2 冻结前完成，并记录环境 identity；准备动作不能冒充 gate PASS。优先复用同一代码
状态且严格串行的已有 Cargo 产物，或选择有明确磁盘预算的隔离 target；不得让不同 worktree/代码状态并发污染
共享 target。node_modules 来源也必须在冻结前验证，不能等 `checks` 执行到 `local-instance` 才临时修复。

## 合流、combined probe 与新稳定周期

唯一集成 Agent `/root/rust_test_integrator` 串行接收 R1 和任何获授权的 R2，核对提交/tree、写集与证据后合流。
在合流后的同一精确代码状态上，由集成 owner 运行一次便宜 combined probe：

1. 静态确认三个函数各自只在权威 owner 出现一次，函数名/属性/函数体双射不变；
2. `cargo test --package skiff-runtime-service-db --lib -- --list`，确认 crate 编译、原根 102 与唯一 live ignore 身份；
3. 若 R2 获授权并合流，执行 `cargo fmt --all -- --check`；始终执行 `git diff --check` 和阶段写集审计。

probe 失败直接退回对应修复 owner，不消耗正式 verdict。probe 通过且 P1 的环境准备固定后，集成 owner才冻结新的
stable candidate E2，并记录精确 commit/tree。E2 建立新的 stability epoch：

- 原 F owner可以仅复验自己第一次 verdict 的精确 blocker，这是 workflow 允许的窄复验；仍需给 E2 单一 PASS/FAIL。
- G 必须由新的独立 gate owner在同一 E2 上运行 compiler、runtime、rust-quality、checks 四个 selector并汇总唯一
  证据；不得沿用旧 E 的未完成/失败结果。
- release 仍要求 F 与 G 对同一 E2 都 PASS。

本记录对应**第一次 F FAIL**和**第一次 G FAIL**。同一验收批次尚未发生第二次 FAIL，也没有同一真实路径在修复后
连续暴露第二个新 blocker，因此当前没有触发收敛熔断。若 E2 再次产生第二次 F FAIL 或 G FAIL，应先按 workflow
做有界闭合审计和批量 DAG 更新，不得直接进入第三轮逐项修复/完整 verdict。

## 明确不修

本批次冻结 non-blocking 扩张，不修改 Service DB result 的行数陈述、Service DB leaf 的 `in progress` 状态或
callable leaf 的入口路径笔误；也不修其它文档、重构生产实现、增加测试、调整 4073 line gate、运行 live Mongo、
新增依赖/配置/兼容层或 push。以上事项不能延长当前 blocker 收敛链。
