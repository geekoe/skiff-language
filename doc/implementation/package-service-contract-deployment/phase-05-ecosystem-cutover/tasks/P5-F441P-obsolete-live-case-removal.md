# P5-F441P Obsolete live case removal

状态：Ready。执行用户决策：删除两条无价值case，不增加测试语言语义。

## 直接父节点

- `P5-F441M-live-test-execution-gap-audit-result.md`
- `P5-F441I-canonical-live-source-root-authoring-result.md`

F441M已证明`__skiffPayload`与expected-platform-error都没有current语法/runner表示，并记录用户决定：
不新增`input`、`expects platformError`或其它兼容路径，直接删除两条case和专用helper。F441I拥有真实root
compile/receipt test。

实现基线为`c5777f16`对应的current integration tree。

## 目标

删除：

1. `runtime/live-tests/internal/operation.live.test.skiff`中的
   `live operation dispatch crosses runtime binary payload boundary`；
2. `runtime/live-tests/internal/file_live.live.test.skiff`中的
   `live file runtime rejects stream above file guard limit`；
3. 只为上述case存在、全仓无其它consumer的normal-source helper。

更新真实root integration，使剩余operation case与其它runtime-live cases全部真正compile，不再保留
“discover但later execution owner再处理”的旁路。其它file lifecycle cases保持不变。

## 唯一写集

- `runtime/live-tests/internal/operation.live.test.skiff`
- `runtime/live-tests/internal/file_live.live.test.skiff`
- `runtime/live-tests/internal/operation.skiff`
- `runtime/live-tests/internal/file_live.skiff`
- `test-runner/tests/package_service_contract_deployment.rs`中canonical live source root test及直接helper
- 本leaf result

禁止修改syntax、testing reference、test-runner production、Router/Runtime production、HTTP manifests、
scripts、其它fixture/task/result。不得派子Agent，不得运行live/stable/instance。

## Test-first与完成标准

先把真实root test改为compile所有remaining tracked case，并加入反向断言，使旧source至少因
`__skiffPayload` unresolved或obsolete case/helper仍存在而失败，再删source。

终态必须证明：

- operation只有runtime-owned fixture case且可正常compile；
- file只保留三个现有lifecycle case且全部compile；
- DB/HTTP case不变；
- default encrypted case与全部runtime-live remaining cases的pure compile总数按实际source精确记录；
- canonical package/service/deployment/gateway receipt完全不变；
- `__skiffPayload`、over-limit case title、专用helper与“later execution owner/discovered only”断言为零；
- normal-source helper只在确认全仓无其它consumer后删除。

必跑：

```bash
cargo test -p skiff-test-runner --test package_service_contract_deployment \
  canonical_live_source_roots
cargo test -p skiff-test-runner --test package_service_contract_deployment
cargo fmt --all -- --check
git diff --check
```

Cargo统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

反向搜索：

```bash
rg -n '__skiffPayload|liveFileOverLimitChunks|payloadRoundTrip|later execution|discovered only|rejects stream above file guard limit' \
  runtime/live-tests test-runner/tests/package_service_contract_deployment.rs
```

若`payloadRoundTrip`或`liveFileOverLimitChunks`存在其它真实consumer，保留该helper并在result分类；不得为
追求零匹配删除其它行为。

## 停止与交付

若剩余case compile暴露与本次删除无关的test-runner production blocker，提交source删除与测试预期中仍有效
部分，返回`TASK_SCOPE_EXPANDED`；不得修改runner。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f441p-obsolete-live-cases`
- branch：`codex/p5-f441p-obsolete-live-cases`
- result：`P5-F441P-obsolete-live-case-removal-result.md`

Implementation与result分开提交；不merge/rebase/push。
