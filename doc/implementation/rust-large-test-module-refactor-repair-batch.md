# Rust 大型测试模块重构：第一次 F/G 失败后的修复批次

日期：2026-08-01

状态：repair DAG ready；R2 ready

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
R2 linker 单文件机械 rustfmt ──────────────┘                    ├─> 同一 F 只复验精确 blocker
                                                               └─> 新 G 完整 gate
```

2026-08-01 用户已经明确授权 R2，授权边界已写入唯一 authority 的同日修订。该 authority 修订合流后，R1、R2
与 P1 可以并行推进；只有 R1 和 R2 均已合流、P1 完成，才能运行 combined probe 并冻结 E2。

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

状态：ready；2026-08-01 用户已授权。

唯一 authority 的 2026-08-01 用户授权修订把
`runtime/linker/src/assembly/tests/cross_package_actor.rs` 纳入唯一例外写集。该文件是现有 test-only linker 测试，
初始 baseline、E 与本批次当前集成 baseline `45a59a38f674bc0aa5e55f0b96c49205b73406d3` 都是 blob
`8bbe074b19c47f6178298a5a59950e876baeea86`；本修复只为解除第 4 节第 4 项既有 workspace rustfmt blocker。

新的 R2 Agent 必须从派发时最新集成 baseline 做零-worktree 预检：确认精确 commit/tree、目标 blob、主工作区与
集成 worktree 的 dirty 集合，以及运行中兄弟任务的 ownership；若目标 blob 已变化、目标文件已 dirty 或存在并发
owner，立即停止并上报，不得覆盖他人改动。预检通过后才创建最小独立 worktree，以仓库 edition/config 对该文件
单独运行 rustfmt。静态 diff 必须证明唯一改动文件就是该路径，且变化全部为格式；函数、属性、测试逻辑与 linker
行为必须不变。R2 自验收包含单文件 rustfmt check、`cargo fmt --all -- --check`、`git diff --check` 和精确写集证据，
不得顺手修改其它源码、文档或格式文件。

## P1：gate 环境预检与准备

状态：ready；与 R1/R2 并行，但不产生候选 PASS 证据。

P1 先只读确认以下事实并形成可执行准备方案：

- 是否存在可安全复用、与待测精确代码状态一致的 Cargo target；不同代码状态不得并发写同一 target。
- 当前磁盘余量、预计 runtime selector 峰值、明确 target/cache 路径及清理 owner，避免再次在约 6.1 GiB 后
  `ENOSPC`。
- `node_modules` 的可信来源与 `yaml` 的可导入性；detached gate worktree 不得再假设依赖天然存在。
- 四个 selector 的展开命令、工作目录、源码 identity 与缓存隔离方式。

必要的依赖/缓存准备必须在 E2 冻结前完成，并记录环境 identity；准备动作不能冒充 gate PASS。优先复用同一代码
状态且严格串行的已有 Cargo 产物，或选择有明确磁盘预算的隔离 target；不得让不同 worktree/代码状态并发污染
共享 target。node_modules 来源也必须在冻结前验证，不能等 `checks` 执行到 `local-instance` 才临时修复。

2026-08-01 用户另行授权删除可重建的 `/Users/geek/workspace/skiff/target` Cargo cache，作为 P1 解除磁盘空间
blocker 的外部 gate 环境准备路径。P1 owner 执行前仍须核对精确路径、其可重建/ignored 身份和进程占用，且不得
触碰 stable instance 使用的 `/Users/geek/workspace/skiff/build/cargo-target`。该授权不进入 repo 写集，不是代码
完成标准，也不能冒充 gate PASS。

## 合流、combined probe 与新稳定周期

唯一集成 Agent `/root/rust_test_integrator` 串行接收 R1 和 R2，核对提交/tree、写集与证据后合流。
在合流后的同一精确代码状态上，由集成 owner 运行一次便宜 combined probe：

1. 静态确认三个函数各自只在权威 owner 出现一次，函数名/属性/函数体双射不变；
2. `cargo test --package skiff-runtime-service-db --lib -- --list`，确认 crate 编译、原根 102 与唯一 live ignore 身份；
3. R1 与 R2 均合流后执行 `cargo fmt --all -- --check`；始终执行 `git diff --check` 和阶段写集审计，并确认
   authority 的例外写集只出现该 linker test 文件的机械格式变化。

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
