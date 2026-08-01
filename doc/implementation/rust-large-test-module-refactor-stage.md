# Rust 大型测试模块重构：阶段 DAG

日期：2026-08-01

状态：ready

权威设计：[`rust-large-test-module-refactor.md`](./rust-large-test-module-refactor.md)，baseline commit
`2b584edc6c30cec4acf8fd2c9e3bde85790fe234` / tree
`d6e2bb3f62d48e10810b688791833cb22d189354`。本文只定义执行拓扑与 ownership；任何语义取舍回到权威
设计，不在叶子任务中另立方案。

## DAG

```text
A callable-effects dev ─┐
                        ├─> C integration checkpoint ─> D line gate ─> E stable candidate ─┬─> F independent acceptance
B service-db dev ───────┘                                                                  └─> G independent gate

release = F PASS && G PASS，且两者审查的是同一个 E commit/tree
```

A、B 可并行；C 之前不得互相 cherry-pick。D 只能基于 C 的联合结果计算真实最大值。E 固定后禁止再写；
F、G 使用不同于 A–E 的 agent，均只读。任何失败回到唯一 owner 修复，产生新候选后 F、G 都重跑。

## 节点与不重叠写集

| 节点 | worktree / branch | 唯一写集 | 交付 |
| --- | --- | --- | --- |
| A | `/Users/geek/workspace/skiff-rust-test-callable` / `codex/rust-test-callable` | `compiler/source/src/callable_effects/tests.rs`、`compiler/source/src/callable_effects/tests/**`、`doc/implementation/rust-large-test-module-refactor-callable-leaf.md`、`...-callable-result.md` | 单个可 cherry-pick commit（必要时保持短 commit 串），focused 证据 |
| B | `/Users/geek/workspace/skiff-rust-test-service-db` / `codex/rust-test-service-db` | `runtime/service-db/src/tests.rs`、`runtime/service-db/src/tests/**`、`doc/implementation/rust-large-test-module-refactor-service-db-leaf.md`、`...-service-db-result.md` | 单个可 cherry-pick commit（必要时保持短 commit 串），focused 证据 |
| C | `/Users/geek/workspace/skiff-rust-test-integration` / `integration/rust-large-test-module-refactor` | 只 cherry-pick A/B；任何源码冲突或修复都退回 A 或 B | 联合测试通过的 checkpoint commit/tree 与写集审计 |
| D | `/Users/geek/workspace/skiff-rust-test-line-gate` / `codex/rust-test-line-gate`（从 C 创建） | `scripts/check-rust-file-lines.mjs`、`doc/implementation/rust-large-test-module-refactor-line-gate-result.md` | 真实最大值、降序清单、checker 结果与 commit |
| E | C 的 integration branch | 只 cherry-pick D；不得直接编辑 | 固定 stable-candidate commit/tree |
| F | `/Users/geek/workspace/skiff-rust-test-acceptance`，detached at E | 无 | 逐条验收权威设计并 PASS/FAIL |
| G | `/Users/geek/workspace/skiff-rust-test-gate`，detached at E | 无 | 运行权威设计第 6 节 gate 并 PASS/FAIL |

开发节点只能在自己的 worktree 写自己的源码写集和 leaf/result 文档；不得编辑本阶段文档、另一开发域、
line gate 或生产代码。`tests/**` ownership 以表中前缀区分，因此 A、B 没有共享文件。协调父节点负责记录
agent、branch、worktree、baseline、commit/tree 和证据，不让开发节点直接修改集成分支。

## 节点入口/出口

### A / B：开发叶子

入口必须确认 HEAD/tree 精确等于本阶段 baseline，并先把测试全名、数量和 ignored 状态写入 leaf 文档。
实现严格使用权威设计的终态职责与复杂度约束。result 文档记录实际文件映射、前后测试双射、命令与退出
码；源码与 result 一起提交。禁止新增依赖、配置、框架或改动生产可见性。

### C：集成检查点

按 A 后 B（或 B 后 A）cherry-pick 均应无重叠源码冲突。C 运行两域 focused tests、全量 test list
对照、ignore 审计、rustfmt，并用 `git diff --name-only <baseline>...HEAD` 验证写集。C 不降低 line gate，
也不代替独立验收。

### D：line gate

在 C 的精确 tree 上列出全部 tracked Rust 文件行数，取真实最大值及其路径；把
`MAX_FILE_LINES` 设为该数值并同步 `current maximum` 注释。不得预设 4073 等候选值，不得新增例外或改
checker 算法。更新前后都保留命令输出，更新后 checker 必须通过。

### E / F / G：候选与双门禁

E 记录合入 D 后的不可变 commit/tree。F 按设计完成标准做只读结构/测试身份审查；G 在同一 tree 上运行
规定 selector 和静态 gate。F/G 任一失败都不能声明完成；修复必须回到对应 owner 分支并形成新的 E，
旧验收证据随之失效。

## 停止条件

只有 A、B、C、D 均有可追溯提交和证据，E 已固定，且 F、G 对同一 E 均 PASS，父节点才可合流或向用户
报告完成。不得 push；不得修改、清理或吸收主工作区已有未跟踪文件。
