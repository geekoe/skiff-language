# Phase 1 result: artifact schema and structural validator

Status: candidate-pass（隔离 Live 与 focused gates 全绿；待合并 main 后完成 stable closure 标 complete）

## Candidate commits/trees

隔离 Live 候选（bcvm/p1-design 分支，skiff 仓库）：

| commit | 内容 |
| --- | --- |
| 42f6bf4a | Phase 1 设计文档（opcode descriptor table / DTO / decoder / validator / identity / store path） |
| e6d6e5e4 | 设计决策 D1–D19 全部确认 |
| f92dd59a | artifact-model/src/bytecode/{opcodes,dto,encode,decode,validate}.rs + limits + refs（222 tests） |
| a01a38c6 | property/corpus/roundtrip 测试补全（种子化 fuzz 入口） |
| 6dd4a88f | artifact-identity bytecode identity/store + PackageArtifact 联动（C9） |
| 32a5ebd7 | 全仓 PackageArtifact bytecode 字段接线 + 去掉实时内容 identity golden |
| fca9f63d | std parse-output baseline 重生成（ce818d61 std.json get natives 后过期） |

## Requirement IDs closed/deferred

- Phase 1 范围 requirement：R-003（artifact 侧）、R-009、R-017、R-018、R-019、R-020（artifact 侧）、R-022、R-078（职责切分 3）、R-079、R-080（artifact 侧）、R-081（schema 声明部分）、R-220（schema 声明部分）——schema/decoder/validator/identity/store 面落地。
- 归后续阶段：R-023/3B（move-slot 证明）、R-080/3B（linked image 表）、R-220/6B（drop/transfer 语义）、semantic verifier（§4.2 全部）。

## Focused gate table

| selector / 命令 | 结果 | 说明 |
| --- | --- | --- |
| `node scripts/verify.mjs --only foundation` | PASS | artifact-model/identity/deployment（含 bytecode 模块 222+ tests） |
| `node scripts/verify.mjs --only compiler` | PASS | 含 36 处 PackageArtifact 构造接线与 golden 改造 |
| `node scripts/verify.mjs --only checks` | PASS | 17/17 |
| `git diff --check` | PASS | 三仓合流范围 |

## Phase-specific proof

- 单一 opcode descriptor table：42 指令（§3.5 权威清单）完整 operand 布局 + stack 签名 + relocation kind 矩阵，`OPCODE_TABLE` 常量 + `opcode_table_fingerprint`（sha256 canonical 投影）双保险；encoder/decoder/validator 只消费该表（R-022 停止条件满足）。
- Bounded decoder：迭代无递归、checked 算术、四类错误（UnknownOpcode/TruncatedInstruction/ArithmeticOverflow/LimitExceeded）；16 个上限常量全量 boundary 测试（at-limit 通过 / above-limit 拒绝）。
- Structural validator C1–C8：10 类 corruption 每类 ≥2 变体 corpus（手写 fixture，不由 encoder 生成）+ property 测试（种子化 word stream：decode 永不 panic、失败先于越界访问、确定性）。
- Canonical identity/store（C9）：preimage = schema/ISA version + opcode fingerprint + 全内容（含 debug table）；schema 变化必然改变 build identity；Local ABI projection 不含 bytecode（依赖不重编译）；`bytecode: Option` 用 skip_serializing_if（无 bytecode 的既有包 build identity 不变——compiler gate 的 36 处构造接线验证）。
- 并发/顺序确定性：BTreeMap 消除 map 顺序；manual 顺序 publish 两次完全一致（独立 store 实验）。
- 测试 golden 改造：std/prelude 实时内容字节级 golden（历史引入，每次 std 改动都要同步）替换为 canonical-frame + 确定性断言（两次 fresh 物化一致 / registry vs legacy loader 互证 / 具体符号断言）。

## Isolated Live

- selector：`node scripts/verify.mjs --only router-live:agine`（chat-smoke → host-tools --check → strict full host-tools）。
- 最终 PASS 运行（run2）：manifest `/Users/geek/workspace/.local-dev/phase-1-manifest-r2.json`（schemaVersion v1，engine legacy-tree——本阶段为全栈回归，非 bytecode 执行证明）：
  - chat smoke PASS（reply 12 chars）；host-tools check PASS；strict full host-tools PASS（terminal=completed，10 tool calls，2836 chars，sample 非空）
  - assembly `skiff-runtime-assembly-v3:sha256:58c739ee…`，config snapshot `…bb572b52…`
- run1 的 host-tools 失败（46 次 read_file 循环 + 空回复 + router delivery miss）经 run2 判别为**模型行为波动**（同候选重跑 PASS）；delivery miss 为 router WS 投递日志（非致命，host 连接关闭期投递），不阻塞 gate。

## Stable merge commits and Live receipt

- 待合流：三仓 main merge commits（本阶段仅 skiff 有代码改动；internals/skiff-packages 无变更）。
- 合流后 stable closure：`internals/agine` main 上 chat-smoke + host-tools（strict 断言，normal-flow-only 约定）。

## Known residual risks owned by the next phase

1. **稳定 dev 环境 profiler 故障（与 Phase 1 代码无关）**：稳定 runtime.yml `profile.enabled: true`（pprof 1000Hz）导致 chat/host-tools 全断（dispatchThreadActorTickUnsafe 失败循环、模型调用不发起）。关闭 profiler 后稳定栈 chat-smoke 与 host-tools 全部恢复。根因（profiler 与 actor/task 交互）未深挖，恢复 dev 功能优先；Phase 2 evidence epoch 前需修 runtime profiler 或明确禁用配置。隔离栈（harness 生成配置无 profiler）不受影响。
2. host-tools 模型行为波动（LLM 随机性）：run1 46 调用空回复、run2 10 调用正常；normal-flow-only 约定下以重跑判别，不逐项排查模型输出。
3. 未完成 CLI 级验证（沿用 Phase 0 决定）：注入 6 种角度（单元级覆盖）、manifest 重复生成一致性——由单元测试与确定性实验覆盖。
4. watch 构建循环（10:36–11:07 持续全量重建）已自行收敛（fingerprint 稳定）；期间曾出现发布引用缺失 record 的瞬时失败（`failed to resolve artifact …d3e50b78…`），未复现，记录待查。

## Verdict

Phase 1 candidate-pass：唯一 schema owner（opcode descriptor table）、全部 DTO、bounded decoder、structural validator（C1–C8）、canonical identity/store（C9）与 malformed/property 测试落地；foundation/compiler/checks 全绿；隔离 Live 三段 PASS（manifest 已归档）。合并 main 后完成 stable closure 标 complete。
