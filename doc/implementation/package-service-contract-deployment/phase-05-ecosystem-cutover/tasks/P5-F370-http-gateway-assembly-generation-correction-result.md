# P5-F370 HTTP gateway assembly generation correction result

状态：Completed（仅纠正runtime request HTTP gateway validation中的generation语义；未修改
canonical request wire、ActivationIdentity、Host route、F363 target公共结构或request lifecycle）。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| integration base | `9c668bf9f8aa75f0ebfdf076915d5b5de1c4d327` | `fdf793aed683942f6ffcd783bbb35738e19e3c91` |
| task checkpoint | `2710cf4e9a611e30d7c2d96fd42f15ba42ad4cfb` | `7bc12a5f34b4f48ed97b53e4cbc3fbb7cbc5babb` |
| production/tests | `7ddcbf31b06bdc86212b0f406df55f49c06f1231` | `0d46a1df6811f34da8586a6dac25c1715e0165d5` |

工作分支为`codex/p5-f370-assembly-generation`，worktree为
`/Users/geek/workspace/skiff-p5-f370-assembly-generation`。本leaf没有merge/rebase/push，也没有
启动stable/live进程。

## 2. Generation correction

- `validate_request`现在把wire `routing.assemblyGeneration`与
  `target.eval().activation_context().identity().assembly_generation`逐值比较。
- `target.eval().request_activation().generation()`不再参与HTTP gateway assembly admission；
  request-local generation及`RuntimeHttpGatewayRequestLifecycle`的cancel/end ownership均未修改。
- 既有canonical shape、assembly identity、gateway identity、routing/http method/path及
  surface/plan/mode检查统一进入同一个private validation facts seam；错误消息与fail-closed顺序不变。

## 3. Direct regression

新增4个direct request validation tests：

| 测试 | 证据 |
| --- | --- |
| `runtime_http_gateway_same_pinned_assembly_accepts_consecutive_request_generations` | pinned assembly generation固定为17；连续request-local generation 801/802均通过 |
| `runtime_http_gateway_wrong_assembly_generation_fails_closed` | wire generation偏移1即返回pinned activation protocol error |
| `runtime_http_gateway_wrong_assembly_or_gateway_identity_fails_closed` | valid-but-wrong assembly identity和gateway identity分别拒绝 |
| `runtime_http_gateway_disagreeing_request_metadata_fails_closed` | routing与binary HTTP method/path任一不一致均拒绝 |

fixture显式把request-local generation与pinned assembly generation分离；前者只区分两个连续请求，
不会进入assembly validation facts。

## 4. Verification

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-request http_gateway -- --list` | PASS；8 tests，非零 |
| `cargo test -p skiff-runtime-request http_gateway` | PASS；8/8 |
| `cargo check -p skiff-runtime-request -p skiff-runtime-eval` | PASS；仅linker既有dead-code warnings |
| `rustfmt --edition 2021 --check runtime/request/src/http_gateway_execution.rs runtime/request/src/http_gateway_execution/tests.rs` | PASS |
| `git diff --check` | PASS |
| `git diff --exit-code -- Cargo.lock` | PASS；零差异 |

## 5. 自验收矩阵

| 任务条款 | 代码/测试证据 | 结果 |
| --- | --- | --- |
| assembly generation来自pinned activation | `validate_request`读取`activation_context().identity().assembly_generation` | PASS |
| request generation保持request-local | production不再读取其generation；lifecycle cancel/end未改 | PASS |
| 连续不同request generation均合法 | 801/802在同一assembly generation 17下均通过 | PASS |
| wrong assembly generation fail closed | dedicated generation负例 | PASS |
| identity与metadata负例不退化 | assembly/gateway identity及method/path负例 | PASS |
| selector非零 | list枚举8个，执行8/8 | PASS |
| 窄写入与停止边界 | production/tests仅execution文件与direct fixture；禁止域、lockfile零修改 | PASS |

正确修复不要求修改`RuntimeAssemblyRequestStartFrameHeader`、ActivationIdentity、Host route或F363
target公共结构，因此未触发`TASK_SCOPE_EXPANDED`。
