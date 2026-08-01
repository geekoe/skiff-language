# Rust 大型测试模块重构：linker rustfmt 修复叶子

日期：2026-08-01

状态：completed；等待集成 owner 接收

直接父节点：[`rust-large-test-module-refactor-repair-batch.md`](./rust-large-test-module-refactor-repair-batch.md)。
该父节点经阶段文档引用唯一权威设计
[`rust-large-test-module-refactor.md`](./rust-large-test-module-refactor.md)，其中 2026-08-01 用户授权修订是本节点
写入既有 linker test-only 文件的唯一依据。

## 节点与基线

- DAG 节点：R2（linker 单文件机械 rustfmt）；前置为 authority 修订已经合流，完成后解除 batch combined probe
  与 E2 freeze。
- repo：`/Users/geek/workspace/skiff`。
- baseline：commit `2d040ac9b7324741f4083823178a2f0fd838c16e`，tree
  `8920797830eb9b9a6217514816cc7a6f87b337cb`。
- worktree / branch：`/Users/geek/workspace/skiff-rust-test-rustfmt-repair` /
  `codex/rust-test-rustfmt-repair`。
- 唯一集成 owner：`/root/rust_test_integrator`；本节点不合并集成分支、不 push，合流前保留一级 worktree。
- 当前成熟度：预验收候选上的修复输入；完成后仍需集成 owner 的 combined probe，不能自行冻结 E2。

## 零 worktree 预检

只读预检确认集成分支仍为上述 commit/tree；目标
`runtime/linker/src/assembly/tests/cross_package_actor.rs` 的 baseline blob 是
`8bbe074b19c47f6178298a5a59950e876baeea86`。主工作区、集成 worktree 和已登记的其它 worktree 均未修改该路径，
也没有独立 cargo、rustc 或 rustfmt 进程正在拥有该表面。主工作区现有 Router 脏改动属于其它 owner，禁止触碰。
crate `runtime/linker/Cargo.toml` 声明 Rust edition 2021；baseline 不含额外 rustfmt 配置。

## 写入范围与非目标

唯一源码写集是 `runtime/linker/src/assembly/tests/cross_package_actor.rs`，只允许
`rustfmt --edition 2021` 产生的机械格式差异。证据写集仅为本文件与配套
`rust-large-test-module-refactor-rustfmt-repair-result.md`。

不得修改测试逻辑、函数、属性、linker 行为、生产源码、其它 test 文件、manifest/lockfile、依赖、配置、
callable-effects / Service DB 重构或 line gate；不得格式化其它文件、运行完整 selector 或 live 测试。

## 完成与验证合同

1. 单独格式化目标文件，并以“baseline 内容经过同一 rustfmt 后与结果逐字节相同”证明 diff 完全由 formatter 产生。
2. 比较前后测试函数名、测试属性与完整文件的非格式语义；记录目标 blob、结果 blob 和精确 diff。
3. 运行 `rustfmt --edition 2021 --check <target>`、`cargo fmt --all -- --check`、`git diff --check` 和精确写集审计。
4. result 记录命令、退出码、审计摘要、implementation/result commit/tree 和 clean 状态；提交后直接交接集成 owner。

本节点风险为低：只解除既有 workspace rustfmt gate，不改变用户可见行为或公共契约。证据仅对本分支最终提交的
源码、rustfmt 工具链和上述 baseline 有效；目标文件、属性、manifest、formatter 配置或 baseline 改变都会使相应
证据失效。若目标 blob 漂移、出现并发 owner、formatter 产生超出纯格式的差异，或完成需要扩大写集，立即停止并
报告 `TASK_SCOPE_EXPANDED`。
