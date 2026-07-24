# P5-F141：Contract Stream Call Source Typing

状态：Ready

## 父节点与进入状态

- 直接父节点：`P5-F139-service-stream-boundary-projection-result.md`。
- 父节点向上追溯到 P5-D82 result、审计合同与唯一权威设计。
- 进入 checkpoint：最外层 `Stream<T>` 已生成既有 `ServerStream` contract；Runtime lane 已存在。

## 当前 owner 与遮挡

- source contract call owner：
  `compiler/source/src/expression_type_model/contract_call_typing.rs`。
- 当前 `validate_operation_semantics` 明确拒绝所有非-unary stream，导致真实
  `for event in alias/operation(input)` 无法进入 lowering。
- `compiler/source` 已拥有普通 `Stream<T>` / `for` 的 source type model；本任务只连接既有 server-stream item type，
  不引入新的调用语法或 generic operation type arguments。

## 写入范围

- `compiler/source/src/expression_type_model/contract_call_typing.rs`
- `compiler/source/src/expression_type_model/contract_call_typing/tests.rs`
- 必要的同目录小型 helper。
- 禁止修改 parser、artifact/contract schema、lowering、Runtime、HTTP boundary。

## 完成标准

1. `BoundaryStreamContract::ServerStream { item_type, .. }` 的 contract call expression 具有 canonical source
   `Stream<item>` 类型，因此能被现有 `for` 顺序消费并保持 nominal ContractTypeId。
2. 调用参数、arity、alias/operation identity 与 source generic prohibition 保持现状。
3. unsupported callback/error/cancellation 继续 fail closed；不得把 stream 当 unary final return。
4. 至少覆盖 builtin item、contract nominal item、错误消费类型和显式 source type arguments 负例。

## 验证与证据

- 运行 contract call typing 聚焦测试，selector 必须实际列出并运行目标测试；零测试无效。
- 运行目标文件格式和 `git diff --check`；不运行完整 gate。
- 风险：高，共享 source typing checkpoint；证据对最终提交有效，contract stream/item projection 变化即失效。
- 若必须定义新 stream lifecycle、错误语义或调用语法，返回 `TASK_NOT_EXECUTABLE`。

## Worktree

- `/Users/geek/workspace/skiff-p5-f141`
- branch `codex/p5-f141-contract-stream-source-typing`
- 新的一次性开发会话；提交、不 push、不操作 stable。

