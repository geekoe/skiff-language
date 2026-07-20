# P4-F13：Typed Full-Chain Provider Fixture Completion

## Blocker、输入与边界

T10在exact clean candidate `453c11f0c809cf0af6788375988eb973237e5aaa`运行runtime gate时，
`typed_execution_fixture_uses_projected_admitted_targets`到达真实provider后报
`RuntimeProgram executable provide missing block entry`。typed artifact fixture的plain unary provider仍使用
`ExecutableBody::default()`，而断言还期待早期`service-call` checkpoint；这不再能证明Phase 04要求的真实
typed artifact→projection→link/admit→provider execution链。

权威输入为架构§2、§6–§10、§12、§14，P4-T03、T04、T07与T10合同。只完成typed execution fixture及断言；不得
回退成手写resolved target、恢复checkpoint error或修改production来接受空executable。

- 依赖：T10@`453c11f` runtime gate FAIL。
- 解锁：T10 retry与A01 full-chain证据。
- branch：`codex/p4-f13-typed-provider-fixture`。
- worktree：`/Users/geek/workspace/skiff-p4-f13-typed-provider-fixture`。
- integration边界：只提交task branch，不merge integration/main、不push。

## 写入范围与完成态

独占`runtime/host/src/loader/assembly_admission/tests/execution/{artifacts,scenario,ordinary,async_stream_cancel}.rs`中
provider artifact body、scenario assertion与必要test-only helper。不得修改host普通request tests、callback/native
专项fixture、runtime/linker/eval production或compiler。

1. fixture用显式provider behavior/role表达plain unary provider，不再用新增bool或`service_call.is_none()`隐式推断；
   provider拥有最小有效typed `entry`并返回例如`bool(true)`。consumer继续从真实`ServiceContract`、
   `PackageArtifact`、`ServiceDeployment`经过projection、resolver、typed load/link/admit取得target。
2. consumer service-call与package-direct checkpoint用`Return`传播结果；typed fixture checkpoint、ordinary
   service/package对照、async owned-provider future和ingress/internal dispatcher probe升级为最终成功断言，不再把
   checkpoint/handoff error当成功证据。不得手写resolved provider或直接构造dispatcher target。
3. package-direct、ordinary/error、async/stream共用fixture的既有证据保持，callback/native专项filter保持；空/missing
   block仍在专属validation负例fail closed。
4. production diff必须为空；反向搜索fixture中的`ExecutableBody::default()`，仅允许明确测试empty-body失败的负例。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-host typed_execution_fixture_uses_projected_admitted_targets
cargo test -p skiff-runtime-host typed_execution_ordinary
cargo test -p skiff-runtime-host typed_execution_async_stream_cancel
cargo test -p skiff-runtime-host typed_execution_callback_native
cargo test -p skiff-runtime-host in_process_request_entry
cargo test -p skiff-runtime-host active_generation_context
git diff --check
```

每个filter必须非空。另报告typed artifact到真实provider result的关键symbol链和production零修改证据。

## 回报

提交一个clean commit，回报fixture body、真实结果/隔离断言、负例保留、命令与自验收矩阵。若最小有效body无法经
canonical projection/linker表达，报告精确架构缺口，不得绕过validator。
