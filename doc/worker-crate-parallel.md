# 工兵 crate 并行开发流程（一个工兵一个 crate）

> 状态：生效（2026-08-09，用户确认）。适用本仓库（skiff）与使用同一多 agent 工作流的其他仓库
> （codex 等）。流程与工具无关：本仓库用 opencode 工兵（`multi-agent-development.md`），codex 用其
> subagent 机制时按同一任务书模板执行。

## 1. 为什么

多工兵并行开发时，写界冲突是最大风险（本仓库 Wave 1 曾发生两个工兵互相覆盖同一批文件）。
按 **crate 边界拆分写界** 后：

- 每个工兵只写自己 crate 的源码文件，天然无文件级冲突；
- 每个工兵可以用 `cargo test -p <自己的crate>` 独立自我验收（编译与测试都只涉及该 crate + 依赖）；
- 跨 crate 接口先在设计文档冻结（“接口先行”），工兵按契约实现，接口一致性由主 agent 合流时验证。

## 2. 核心原则

1. **一工兵一 crate**：写界 = 该 crate 的源码文件（`<crate>/src/**`、该 crate 的测试文件）。
   例外必须写进任务书（如跨 crate 的接口改动、`Cargo.toml` 依赖调整、共享 `artifact-model` 的字段）。
2. **接口先行**：任何跨 crate 接口（类型、函数签名、DTO 字段、serde 形状）先由主 agent 写入
   权威设计文档（如 `doc/implementation/bytecode-vm/design/`），工兵只能按文档实现，
   不得自行设计跨 crate 契约；发现契约缺口 → 上报，不自己补。
3. **同 crate 内多工兵**：按文件界拆分（如 `src/mir/**` vs `src/const_evaluator.rs`），
   `lib.rs`/`mod` 接线归主 agent 合流时做，工兵不碰。
4. **主 agent 唯一合流写者**：`cargo check --workspace`、git 提交顺序、跨 crate 集成由主 agent
   串行处理；工兵只提交自己写界内文件的里程碑 commit。

## 3. crate 地图（唯一归属，勿自创分组）

crate → verify subject 的唯一归属声明在 `scripts/lib/verify-rust-subjects.mjs`
（新增 workspace crate 必须归入恰好一个 subject）。常用分组：

| subject | crate（包名） | 职责 |
| --- | --- | --- |
| foundation | canonical-json、artifact-model、artifact-identity、deployment、runtime-config-snapshot、config-snapshot-tooling、syntax | schema/DTO/identity/store/语法（最大共享面，改动影响所有下游） |
| compiler | compiler/{core,contract,input-model,input,source,lowering,compiled,projection-input,projection,emission} + compiler（driver） | 检查、lowering、投影、emission、编译管线 |
| runtime | runtime/**（eval、host、linker、loader、model、native、transport、request 等）+ profiling | 执行、链接、宿主集成 |
| test-runner | test-runner | 测试编译与隔离执行 |
| router | router（skiff-router）、task-control | 路由与任务控制 |

依赖方向要点（写任务书时核对 `Cargo.toml`，防环）：

- `compiler/lowering` → source、syntax、artifact-model；`compiler/emission` → lowering、projection、artifact-identity（新增依赖时确认无环）。
- 编译器 crate 全部依赖 `artifact-model`：共享字段改动 = 全 compiler subject 重编译，宜由主 agent 先落定。

## 4. 任务书模板（主 agent 派发时填写）

```text
目标与写界：
  - crate：<包名>（路径）
  - 写界文件清单：<crate>/src/** 等；例外：<跨 crate 项>
  - 预期行为：<引用设计文档章节>
  - 自验收命令：cargo test -p <包名>（+ cargo clippy -p <包名> --all-targets 可选）
  - 合流验收（主 agent 跑）：cargo check --workspace；node scripts/verify.mjs --only <subject>；--only rust-quality
预算：工具调用次数上限 / 总时长 / bash 超时
禁止：不建 worktree/分支、不改清单外文件、不跑清单外验证、不 push、不 cargo clean、不并发跑 cargo
上报阈值：需要改清单外文件；发现契约缺口；行为与设计文档不符
里程碑：实现完成时提交一次
```

## 5. 验收分层

| 层 | 内容 | owner |
| --- | --- | --- |
| L0 | `cargo test -p <自己crate>`（工兵自验收，预算内） | 工兵 |
| L1 | `cargo check --workspace`（跨 crate 接口）+ git 写界/提交核对 | 主 agent（每次合流） |
| L2 | `node scripts/verify.mjs --only <subject>`、`--only rust-quality`、`git diff --check` | 主 agent（每 wave 结束） |

**注意触发时机**：`cargo test`/`cargo build` 不会执行 clippy lint。workspace `[lints]` 里的
`clippy::too_many_lines`（deny，阈值 534）、`clippy::tests_outside_test_module` 等只在
`cargo clippy`（verify 的 `rust-quality` task）时真正生效。因此：

- 工兵只跑 `cargo test -p` 不能证明 clippy 规则通过；
- 工兵自验收建议补一次 `cargo clippy -p <包名> --all-targets`（预算允许时）；
- L2 的 `--only rust-quality` 是唯一权威 gate，行数/测试位置违规在合流时暴露。

## 6. 纪律（任务书未重复时也默认生效）

- **cargo 串行 + 禁 clean**：多 worktree 共享 `~/.skiff-cargo-target`，cargo 命令排队执行；
  `cargo clean` 会清掉共享产物，禁止。
- **长命令重定向**：可能超过 ~30 秒的命令输出重定向到临时文件再 grep，不重复运行取输出。
- **提交纪律**：只 `git add <写界文件>`；不 push/merge/pull；main 工作目录不 checkout 其他分支；
  提交前 `git status` 核对；他人未提交改动不碰并上报。
- **读仓库 AGENTS.md**：工兵开工先读目标仓库 `AGENTS.md`（测试入口、开发约定）；与任务书冲突时
  以任务书为准并上报。
- opencode 工兵的角色定义（预算、超限处理、报告格式）见
  `/Users/geek/workspace/.opencode/agent/worker.md`；codex 下把同一套约定写进 subagent 提示词。

## 7. 报告格式（固定）

```text
{完成了什么, 意外点, 尝试过什么, 需要什么}
```
