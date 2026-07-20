# P4-F12：Canonical Host Request Fixture Migration

## Blocker、输入与边界

T10在exact clean candidate `453c11f0c809cf0af6788375988eb973237e5aaa`运行runtime gate时，
`skiff-runtime-host --lib`有20个既有`host::tests`失败。T07已让production request entry只接受canonical
`IngressSelector`与active assembly route，但旧共享test helper仅填入`ingress_selector: None`且未发布admitted
assembly，导致这些测试在到达各自原断言前统一fail closed。

权威输入为架构§2.6、§2.8–§2.10、§6.2、§12、§14，P4-T07、R03与T10合同。只迁移host test fixture和
原测试调用；不得恢复selector缺省、legacy route registry、build/operation/display fallback或request-time artifact load，
不得修改production。

- 依赖：T10@`453c11f` runtime gate FAIL。
- 解锁：T10 retry。
- branch：`codex/p4-f12-canonical-host-fixture`。
- worktree：`/Users/geek/workspace/skiff-p4-f12-host-fixture`。
- integration边界：只提交task branch，不merge integration/main、不push。

## 写入范围与完成态

独占`runtime/host/src/host/tests.rs`中的共享request fixture/helper及其20个调用点；若必须拆分聚焦test helper，可在
同一test-only目录新增模块。不得修改`request_entry`、loader production或typed full-chain fixture。

1. 建立一个共享且明确test-only的resolved-spawn helper：复用现有`lookup_request_operation`和
   `spawn_resolved_request`，保留`runtime.request_error`事件名；14个实际callsite（含两个覆盖8项测试的helper）统一
   走它，不能逐处复制实现。
2. 20个测试继续验证各自原有binary/http/telemetry/error语义；invalid symbol、unlinked response/call target等负例
   必须到达原失败点，不能把预期改成canonical入口失败而假绿。
3. 不给只含旧`RuntimeConfig.services`的fixture伪造canonical selector/assembly。缺selector、无active assembly、
   歧义selector的T07 production负例继续直接调用`spawn_request`并fail closed；即使旧registry存在匹配项也不得命中
   test helper或fallback。
4. 反向搜索14个旧`host.spawn_request` callsite，production入口只由canonical/负例测试直接使用；production diff必须
   为空。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-host host::tests::
cargo test -p skiff-runtime-host in_process_request_entry
cargo test -p skiff-runtime-host active_generation_context
cargo test -p skiff-runtime-request assembly_ingress
node scripts/check-runtime-execution-boundaries.mjs
git diff --check
```

每个filter必须非空。另用`git diff -- runtime/host/src/host/request_entry runtime/host/src/loader`证明未改production，并
回报20个原失败测试全部通过、T07负例仍通过。

## 回报

提交一个clean commit，回报共享fixture owner、20项迁移结果、fail-closed负例、反向搜索与自验收矩阵。若必须改变
production才能保持原测试语义，立即报告`TASK_NOT_EXECUTABLE`，不得加入compatibility path。
