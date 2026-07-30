# P5-F140B：Service Stream Host Full-chain Probes

状态：Ready（替代 owner 错配的 P5-F140）

## 父节点与 DAG

- 直接父节点：`P5-D82-service-call-stream-capability-audit-result.md`。
- 父节点已向上引用审计合同和唯一权威设计；需要公共语义依据时沿该链读取。
- 节点：C1 Runtime 证据 D；前置 P5-D82、P5-F138。
- 完成后解除：AIHub → Agine consumer 重验的 Runtime 证据。

父节点确认 production lifecycle owner 在 `runtime/eval`，但真实 admitted/full-chain fixture owner 在
`runtime/host/src/loader/assembly_admission/tests/execution/`；本任务必须使用后者，不能构造 eval-only runtime。

## 写入范围

- `runtime/host/src/loader/assembly_admission/tests/execution/async_stream_cancel.rs`
- 必要时 `artifacts.rs`、`runtime.rs` 测试 fixture。
- 只有精确证明 provider task cleanup 无现有观测点时，允许在
  `runtime/eval/src/assembly_execution/{async_stream_cancel.rs,mod.rs}` 和 `runtime/eval/src/lib.rs` 增加
  `test-support` counter accessor/re-export。
- 不改 wire、公共生命周期、compiler 或 production dispatch。
- 禁止修改其他 Runtime 测试或因 workspace formatting 产生无关文件漂移。

## 完成标准

- 复用 `TypedExecutionFixture`，经过 admitted resolved binding 与 `execute_runtime_assembly_addr` 的真实 Host dispatcher；
  不构造 eval-only runtime。
- 在现有 normal end、early break、callback lifetime、registry cleanup 基础上，补齐可执行的双 item 顺序、
  generic substitution、provider error、request cancel 与目标 stream 隔离。
- 只对现有 fixture 无法观测且不影响阶段风险的项给出明确证据，不为覆盖率改 production API。

## 验证

- Runtime host execution/stream 聚焦测试、目标文件格式与 `git diff --check`；不运行完整 gate。
- selector 必须列出并实际运行目标测试，零测试不算证据。
- 若公共语义或跨 owner production 修复必需，返回 `TASK_NOT_EXECUTABLE`。

## Worktree

- `/Users/geek/workspace/skiff-p5-f140b`
- branch `codex/p5-f140b-service-stream-host-probes`
- 当前 worktree 包含前一会话未提交的六文件 recovery patch，范围精确为上述三个 Host 文件与三个允许的 Eval
  test-support 文件；七个误格式化文件已经由主 Agent 按精确清单还原。
- 新开发会话必须把现有 patch 当作未验证输入，先读 diff 判断是否满足本合同，不依赖聊天交接。已知最后一个
  provider-error probe 修改后尚未重跑；不得沿用此前部分 PASS 作为最终证据。
- 完成后提交，不 push、不操作 stable。
