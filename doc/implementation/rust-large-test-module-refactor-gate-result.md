# Rust 大型测试模块重构：独立 gate 结果

日期：2026-08-01

状态：FAIL（第一次 G verdict）

Gate 对象与 F 相同，均为稳定候选 E：commit
`0a94d75b3d916e87ff2b0c3ea32bebcbda4fc4fe`，tree
`2f98ed10a01a4f6d8b945d34b9af5d885e797d88`。命令集合来自
[`rust-large-test-module-refactor-stage.md`](./rust-large-test-module-refactor-stage.md) 所引用的
[`rust-large-test-module-refactor.md`](./rust-large-test-module-refactor.md) 第 6 节。本结果不得解释为 E 已通过
未完成的 selector。

## Gate matrix

| 权威命令 | 结果 | 证据与归因 |
| --- | --- | --- |
| `node scripts/verify.mjs --only compiler` | PASS | compiler selector 完成且通过 |
| `node scripts/verify.mjs --only runtime` | INCOMPLETE / G FAIL | 隔离 Cargo target 增长到约 6.1 GiB 后遇到 `ENOSPC`，selector 没有完成；不能记 PASS，也没有形成 candidate regression 证据 |
| `node scripts/verify.mjs --only rust-quality` | FAIL | workspace rustfmt 在 baseline 既有 `runtime/linker/src/assembly/tests/cross_package_actor.rs` 上失败；同一 task 内 file-lines PASS：1392 个 Rust 文件、limit 4073 |
| `node scripts/verify.mjs --only checks` | FAIL（17 / 18） | 17 项通过；`local-instance` 在 detached worktree 中没有 `node_modules`，无法导入 `yaml` package，属于 gate 环境/依赖来源未准备，而非候选行为失败 |

## Verdict 与归因边界

G 必须判定 **FAIL**：runtime selector 未完成，rust-quality 与 checks 也没有达到全绿。已有结果中没有候选引入的
compiler/runtime/test regression 证据；但“未发现 regression”不能把基础设施中断、baseline strict failure 或
未执行范围改写成 PASS。

rustfmt 失败的文件在权威初始 baseline 与 E 上都是 blob
`8bbe074b19c47f6178298a5a59950e876baeea86`，因此归因为 baseline 既有状态；它仍受权威完成标准约束。runtime
的 `ENOSPC` 和 checks 的缺失 `node_modules` / `yaml` 则要求在下一稳定候选冻结前完成 gate 环境预检和准备，不能
在冻结候选上边跑边修环境。

G 创建的 detached gate worktree、临时 gate worktree及约 6.1 GiB 隔离 Cargo cache/target 均已清理；没有需要
后续 owner 回收的 gate worktree 或该次 target。G 没有修改候选、提交源码、运行 live Mongo 或声明 release。
