# 工兵 crate 并行开发流程（一个工兵一个 crate）

## 1. 角色与职责

工兵是执行主 agent 派发的**有界实现任务**的 subagent。主 agent 负责任务书、调度、验收和合流，
不代替架构师判断架构问题。架构师负责调查工兵上报的问题，判断现有代码和契约是否足以得出结论，
以及是否需要用户介入。同一批任务尽量复用同一个架构师，使其保留必要的仓库上下文。

简言之：工兵是传感器，主 agent 是编排者，架构师负责技术诊断和架构判断，用户是产品决策的最终
authority。

委派深度只有一层：工兵不能再创建子 agent；架构师由主 agent 派发。

- **权限**：只修改任务书文件清单内的文件；只运行任务书内的验证命令；任务完成时至少提交一次
  （保证会话中断可恢复）；不建 worktree、不 checkout 其他分支、不 push/merge/pull。为说明问题信号，
  可以跨 crate 只读搜索和阅读代码、文档、测试，但不能顺手修改或运行任务书外的构建/测试命令。
- **范围过大**：工兵发现实现方式不成立、依赖接口有问题或需要修改清单外文件时，立即上报。
  继续工作可能固化错误契约或产生错误结果时暂停；否则可以继续不受影响的部分。
- **预算**（任务书写明，缺省用基准）：工具调用次数上限 / 总时长（墙钟）/ bash 每次超时上限。
  超限纪律：bash 超时立即停止该命令不重试；预算将尽停止新动作，带部分交付返回报告。
- **自查纪律**：每 5 分钟或每 10 次工具调用自查剩余预算；过半收敛；连续两次无进展换方向，
  仍无进展则上报，不继续磨。每次自查同时做一次下面的 Problem Signal scan。
- **报告内容**（最后一条消息）：

 完成了什么, 意外点, 尝试过什么, 需要什么, Problem Signal（没有则写“无”）

- **必须上报**：需要修改清单外文件；需要运行清单外验证命令；
  发现与任务书描述不符的行为或缺失的上下文；发现设计、契约或共享面的可疑问题。

### 问题信号的发现与处理

Problem Signal 不是已经证明的架构缺陷，而是“当前理解可能不完整”的具体迹象。不要求工兵脱离当前
任务巡检整个仓库，也不可能穷举所有问题类型；发现应附着在正在开发的需求、直接依赖和实际失败上。

每次预算自查时，工兵快速问一遍：

- 什么结果、依赖或测试行为出乎预期？
- 任务书的哪个假设没有被代码证实，或者已经被反例动摇？
- 为什么一个局部改动开始要求跨 crate 修改、重复转换、复制字段或增加特殊分支？
- 同一概念是否出现多个 owner、多个表示，或者在不同路径含义不同？
- 即使局部测试通过，是否仍无法解释完整行为为什么正确？

这些问题只是发现提示，不是穷举清单。只要有代码、测试、log 或 diff 证据，就可以形成 Problem Signal；
工兵不应因为尚无解决方案、不确定是否重要或担心误报而压下信号。

工兵可以做少量跨 crate 只读取证，确认现象在哪里发生、是否重复、动摇了哪个假设。能清楚描述现象后
就停止调查：不自行扩展成架构研究，不判断优先级，也不顺手修复任务写界外代码。

工兵不负责证明或解决架构问题。发现意外行为、任务假设不成立、重复绕路、共享接口含义不清，
或局部修复可能改变其它路径时，立即向主 agent 报告，不等最终消息：

```text
Problem Signal：观察到什么；代码/测试证据；哪个假设可能不成立；对当前任务的影响；是否建议暂停
```

主 agent 只负责登记、去重和转交，不自行作架构结论。转交材料称为 **Problem Packet**，包含原始
Problem Signal、当前任务书、相关代码/测试/log/diff 和 canonical 文档。主 agent 应保留同一批任务的
原始信号；多个工兵指向同一抽象或 workaround 时，合并进同一个 Problem Packet 交给架构师。

架构师调查后给出两个独立判断：

- **decision authority**：`SELF_CONTAINED`（现有证据可自洽）、`USER_REQUIRED`（需要产品决定）、
  `EVIDENCE_BLOCKED`（关键资料无法取得）或 `NOT_ARCHITECTURAL`（不是架构问题）；
- **urgency**：`BLOCK_CURRENT`（暂停相关任务）、`BEFORE_MERGE`（合流前解决）、
  `FOLLOW_UP`（记录后续）或 `DISMISS`（无需处理）。

架构师报告保持简短：`结论与证据；影响；decision authority；urgency；架构决定 / 最小用户问题 /
缺失资料；建议任务与验证`。架构师默认不写生产实现。主 agent 据此决定继续、记录、请求用户或派发
开发任务。若 decision authority 是 `USER_REQUIRED` / `EVIDENCE_BLOCKED`，且 urgency 是
`BLOCK_CURRENT` / `BEFORE_MERGE`，问题解除前不得固化相关接口或实现。

## 2. 为什么

多工兵并行开发时，写界冲突是最大风险（本仓库 Wave 1 曾发生两个工兵互相覆盖同一批文件）。
按 **crate 边界拆分写界** 后：

- 每个工兵只写自己 crate 的源码文件，天然无文件级冲突；
- 每个工兵可以用 `cargo test -p <自己的crate>` 独立自我验收（编译与测试只涉及该 crate + 依赖）；
- 跨 crate 接口先以**代码形式**冻结（§4），工兵按契约实现，接口一致性由主 agent 合流时验证。
  这是按单个工兵判断的直接依赖条件，不是把整条上游链变成串行波次闸门。

## 3. 核心原则

1. **一工兵一 crate**：写界 = 该 crate 的源码文件（`<crate>/src/**`、该 crate 的测试文件）。
   例外必须写进任务书（如跨 crate 的接口改动、`Cargo.toml` 依赖调整、共享 `artifact-model` 的字段）。
2. **接口形状 = 代码**：跨 crate 接口（类型、函数签名、DTO 字段、serde 形状）必须由
   **已合入 main 的接口代码**体现（§4 接口工兵产出）。行为和架构语义以 canonical 文档、
   架构师结论或用户决定为准。实现工兵只消费已经决定的契约；发现缺口 → 上报，不自己补。
3. **架构问题交给架构师**：工兵负责提供问题信号和证据，主 agent 负责转交和调度，架构师负责判断。
4. **主 agent 唯一合流写者**：`cargo check --workspace`、git 提交顺序、跨 crate 集成由主 agent
   串行处理；工兵只提交自己写界内文件的里程碑 commit。

## 4. 契约与接口工兵

已经决定的公共契约（接口、DTO、serde 形状、签名）**必须用代码体现**，不能只靠设计文档文字：
文字会被各工兵解读出偏差；接口代码（类型定义、`pub fn` 签名、serde 属性、占位实现）是唯一无歧义的
接口形状载体。接口代码不代替行为或产品语义决策；这类问题先走 §1 的问题信号处理。

流程：

1. **接口工兵先行（仅在契约缺口或共享接口需要变更时）**：主 agent 在派发依赖该接口的实现工兵前，
   先派一个接口工兵（或主 agent 自己）把跨 crate 接口以代码形式落地——通常落在**共享 crate**
   （如 `artifact-model` 的 DTO 字段、`compiler/lowering` 的 `mir/**` 类型骨架、
   `compiler/emission` 的 emitter 入口签名）。多个写界不相交的实现工兵可以在接口工兵合入的同时
   继续并行运行；只有直接消费该接口的工兵才需要等待其落地。
2. **接口工兵与实现工兵流程不同**：
   - 写界 = 共享 crate 的接口文件集（类型/签名/桩），不实现业务逻辑；
   - **验收 ≠ 编译通过**：接口代码可以先于依赖方实现合入 main，此时 workspace 整体可能无法编译
     （依赖方尚未实现），这是预期状态；
   - 接口工兵的自验收 = 接口形状审查（类型/字段/serde/签名与设计文档一致）+ 对接口文件集自身
     做隔离语法检查（`rustc --emit=metadata` 或 `cargo check -p <共享crate>` 仅当该 crate 依赖闭包
     完整时）；
   - 接口合入后，依赖该接口的实现工兵按 §6 任务书启动；不依赖该接口的工兵不受影响。
3. **实现工兵启动条件**：它直接消费的接口已以代码形式合入 main（有接口 commit 可引用）；
   任务书引用接口 commit 与文件路径。该条件只针对该工兵直接消费的具体接口；
   写界不相交且不依赖该接口的工兵，可以在契约工兵并行落地时立即启动。

### 并行调度

- 主 agent 应持续派发所有直接依赖契约已合入 main、或正在由并发契约工兵落地的有界工兵；
  实现部分按本节的启动条件在接口代码可引用后开始，其余时间先做契约/接口工作或不相交脚手架。
- 不要等待用户催促后才派发下一批工兵。
- 不要等待整个上游 wave 完成；只按每个工兵直接消费的接口判断是否具备启动条件。
- 唯一串行 owner 是主 agent：workspace 检查、commit 顺序和共享写冲突由其串行处理。

## 5. crate 地图（唯一归属，勿自创分组）

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
- 编译器 crate 全部依赖 `artifact-model`：共享字段改动 = 全 compiler subject 重编译，宜由接口工兵先落定。

## 6. 任务书模板（主 agent 派发时填写）

```text
目标与写界：
  - crate：<包名>（路径）
  - 写界文件清单：<crate>/src/** 等；例外：<跨 crate 项>
  - 预期行为：<引用设计文档章节 + 接口 commit/文件>
  - 架构依据：<canonical 文档 / 架构师结论 / 用户决定；纯局部任务写“无”>
  - 自验收命令（必跑）：
      cargo test -p <包名>
      cargo clippy -p <包名> --all-targets -- -D warnings   （clippy 规则必跑，不是建议）
  - 合流验收（主 agent 跑）：cargo check --workspace；node scripts/verify.mjs --only <subject>；
      --only rust-quality；git diff --check
预算：工具调用次数上限 / 总时长 / bash 超时
禁止：不建 worktree/分支、不改清单外文件、不跑清单外验证、不 push、不 cargo clean、不并发跑 cargo
上报阈值：需要改清单外文件；发现契约缺口；行为与设计文档不符
Problem Signal：按 §1 格式立即上报，不等最终消息
里程碑：实现完成时提交一次
```

## 7. 验收分层

| 层 | 内容 | owner |
| --- | --- | --- |
| L0 | `cargo test -p <自己crate>` + `cargo clippy -p <包名> --all-targets -- -D warnings`（工兵自验收，必跑） | 工兵 |
| L1 | `cargo check --workspace`（跨 crate 接口）+ git 写界/提交核对 | 主 agent（每次合流） |


**触发时机**：`cargo test`/`cargo build` 不执行 clippy lint。workspace `[lints]` 里的
`clippy::too_many_lines`（deny，阈值 534）、`clippy::tests_outside_test_module`、`clippy::disallowed_methods`
只在 `cargo clippy` 时真正生效（verify 的 `rust-quality` task 跑 `cargo clippy --workspace`，
只检查 lib+bin，不带 `--all-targets`——存量测试代码债务不阻塞门禁）。因此：

- 工兵只跑 `cargo test -p` 不能证明 clippy 规则通过，**clippy 自验收是必须项**（L0）；
- `cargo clippy -p <包名> --all-targets -- -D warnings` 覆盖自己 crate 的 lib+bin+测试；

## 8. 纪律（任务书未重复时也默认生效）

- **cargo 串行 + 禁 clean**：多 worktree 共享 `~/.skiff-cargo-target`，cargo 命令排队执行；
  `cargo clean` 会清掉共享产物，禁止。
- **长命令重定向**：可能超过 ~30 秒的命令（cargo test/check/clippy、verify、package publish 等）
  输出重定向到临时文件再 grep，不重复运行取输出：
  `cmd > /var/folders/v2/l4swjmr50s721ntxp56n759h0000gp/T/opencode/<名>.log 2>&1; echo exit=$?`
- **提交纪律**：只 `git add <写界文件>`；不 push/merge/pull；main 工作目录不 checkout 其他分支；
  提交前 `git status` 核对；他人未提交改动不碰并上报。
- **读仓库 AGENTS.md**：工兵开工先读目标仓库 `AGENTS.md`（测试入口、开发约定）；与任务书或
  canonical 文档冲突时暂停相关工作并上报，不自行选择一方覆盖另一方。

## 9. 报告格式（固定）

```text
{完成了什么, 意外点, 尝试过什么, 需要什么, Problem Signal（无则写“无”）}
```
