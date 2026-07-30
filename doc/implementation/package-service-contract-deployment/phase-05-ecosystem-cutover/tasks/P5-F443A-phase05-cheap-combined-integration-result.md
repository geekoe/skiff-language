# P5-F443A Phase 5 cheap combined integration result

状态：`GATE_NOT_EXECUTABLE`

执行时间：2026-07-27 22:47 CST

## 结论

本节点不能给出 `PASS / STABLE_CANDIDATE_READY`。

已实际运行三仓库规定的 non-live combined gate。已到达的候选断言没有失败，但完整 gate
被两个精确的执行缺口遮挡：

1. 规定的 shared `CARGO_TARGET_DIR` 所在 APFS data volume 已满。失败时只剩
   `116`–`124 MiB` 可用空间，volume 为 `100%`，shared target 自身为 `43G`。
   Rust host/package-test/test-runner 与两个 Internals isolated service graph 均在
   compiler 写入阶段收到 `No space left on device`，没有到达对应测试断言。
2. Gate B 的规定命令没有设置 `SKIFF_ROOT`。receipt test 因而把 toolchain 解析为
   `/Users/geek/workspace/skiff` 的 `main@305882351b1e`，而不是本节点的 integration
   Skiff tree；该 main tree 不含 `skiff-package-service-smoke-fixture` bin，bootstrap
   在进入 Registry receipt 断言前以 exit `101` 中止。精确候选的
   `test-runner/Cargo.toml` 确实声明了该 bin。

因此当前证据既不支持冻结 stable candidate，也没有发现可归因于三棵候选代码的断言
回归。不得把环境/调用接线失败误报为候选 `COMBINED_BLOCKED`。

## 精确候选与开始/结束快照

| Repo | Branch | HEAD | Tree | 开始/结束 |
| --- | --- | --- | --- | --- |
| Skiff integration | `codex/package-service-phase-05` | `2e9f086e9599c2c7b334cfacafe672e43db72c7b` | `3704838d9930c1ee76fca9d6fb73d61330486e08` | clean / clean |
| Internals | `codex/package-service-phase-05` | `232094902785c6e725adafa6f4dc42137a1647b4` | `0178f3282eec1c07cdd031a365abd580fa0f204f` | clean / clean |
| skiff-packages | `codex/package-service-phase-05` | `19cfab5dfc827450d37e1a103d21f31f8effa4f0` | `44081bd0498919086c13adea97c07722cb768352` | clean / clean |

Skiff task worktree 与 Skiff integration worktree 开始时均为
`2e9f086e9599c2c7b334cfacafe672e43db72c7b`。相对任务指定 start
`acbf0ab0`，`git diff --name-status acbf0ab0 --` 只有：

```text
A doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F443A-phase05-cheap-combined-integration.md
```

production 内容 bit-identical。三个候选在结束检查中 `git diff --check` 均为零，
`git status --short` 均为空；候选 HEAD/tree 均未变化。

## Gate A：Skiff 共同接线

### Node/checker/corpus

| 命令 | 结果 |
| --- | --- |
| `node scripts/check-skiff-source-layout.mjs` | PASS |
| corpus `--self-test` | PASS；`controls=6`，`rawCases=79` |
| corpus `--combined-probe` | PASS；`probe=activation-parity` |
| corpus `--runtime-wire-self-test` | PASS；`activationFrames=6`，`activationMutations=7`，`requestMutations=115`，`requestRawCases=19` |
| `pnpm --dir router type-check` | PASS |

专用 worktree 初始没有 `router/node_modules`，前三个 corpus 命令最初在加载 `yaml` 时
中止，type-check 最初找不到 `tsc`。确认专用树与 integration tree 的
`router/pnpm-lock.yaml` blob 均为
`d2edd11672dab8c5ba0aa18e6940b504996302ff` 后，临时链接到 integration tree 的
`router/node_modules`，上述四条命令各重跑一次并全部通过。链接已在 result 前移除。

Router `vitest list` 成功列出 8 个文件、169 个非零测试；同一清单执行结果：

```text
Test Files  8 passed (8)
Tests       169 passed (169)
```

### Rust shared target

| 命令目标 | 结果 |
| --- | --- |
| `skiff-runtime-transport runtime_assembly_websocket_jsonrpc` | PASS；8 passed |
| `skiff-runtime-request websocket_jsonrpc_execution` | PASS；2 passed |
| `skiff-runtime-eval runtime_websocket_jsonrpc` | PASS；10 passed |
| `skiff-runtime-host websocket_jsonrpc --no-fail-fast` | NOT EXECUTABLE；编译阶段 ENOSPC，exit 101 |
| `skiff-runtime-package-test ... entrypoint_validation_rejects_non_exact_gateway_facts` | NOT EXECUTABLE；写 fingerprint 时 ENOSPC，exit 101 |
| `skiff-test-runner --test package_service_contract_deployment` | NOT EXECUTABLE；写 fingerprint 时 ENOSPC，exit 101 |

最早遮挡点是 host 命令编译 `skiff-test-runner` / `skiff-runtime-host` 时的：

```text
rustc-LLVM ERROR: IO failure on output stream: No space left on device
```

这遮挡了 Host websocket JSON-RPC、package artifact exact gateway facts，以及
test-runner package/service contract deployment 的规定断言。前三个已通过的 Rust
target 是相邻 wire 探针，但不能替代这三个未执行 target。

## Gate B：skiff-packages

| 命令 | 结果 |
| --- | --- |
| `npm run type-check` | PASS |
| 两个 Registry source/receipt tests | 7 passed，1 failed before receipt assertions |
| `node scripts/test-packages.mjs --list` | PASS |

唯一失败的测试为
`fresh Registry publish and assembly receipts close twenty service operations`。最早失败是：

```text
cargo run --manifest-path /Users/geek/workspace/skiff/test-runner/Cargo.toml \
  --bin skiff-package-service-smoke-fixture ...
error: no bin target named `skiff-package-service-smoke-fixture`
```

`scripts/registry-service-receipt.test.mjs` 在 `SKIFF_ROOT` 未设置时使用
`join(packagesRoot, '..', 'skiff')`。在本 integration worktree 布局下，该默认值指向
非候选 main tree。source test 的其余 7 个断言均通过；receipt workflow 本身没有被
执行，不能据此判断 Registry receipt 候选失败。

offline list 报告 `externalRequests=false`，列出 5 个 package、6 个 service root；
`openai-live` 仅为 `compile-only` 且测试清单为空，没有执行 external/live。

## Gate C：Internals current authoring

四个 service API receipt 文件的联合结果为：

```text
tests 27
pass 25
fail 0
skipped 2
```

两个 skip 均是要求 isolated authoring owner 提供生成 receipt/build records 的既有
条件 skip，不是失败。

`aihub/service` 与 `agine/service` 均使用任务指定的三个 root 和 shared target 启动。
`agine` 的 `prepare-packages --list --fixture-only` 先成功确认：

- Internals `232094902785c6e725adafa6f4dc42137a1647b4`
- Skiff `2e9f086e9599c2c7b334cfacafe672e43db72c7b`
- skiff-packages `19cfab5dfc827450d37e1a103d21f31f8effa4f0`

随后两个 graph 都在第一个 Rust package 编译写入 shared target 时因 ENOSPC 中止，
没有到达 package publish、service build 或 assembly receipt 断言。workflow 使用
`mkdtemp` 下的隔离 `ecosystem-store`；结束后未留下
`internals-canonical-assembly-*` 或 `skiff-registry-receipt-*` 临时目录，也没有读取
或写入 stable artifact/watch registry。

## 精确缺口与后续节点

最小后续节点建议为 `P5-F443A-R1 cheap combined executable resume`，只处理 gate
可执行性并重验被遮挡范围：

1. 由环境 owner 在 gate 之外恢复 shared target volume 的可用空间。本节点没有执行
   整棵或 package-scoped target 清理；`cargo clean --dry-run` 显示该范围会涉及
   `97978` 个文件、`23.6GiB`，会误伤共享历史缓存。为使 result 能够落盘，只精确移除
   本节点在 `22:43` 生成并运行过的
   `skiff_runtime_eval-d47a3a59d9e65bfd` test binary（`99MiB` 可再生成缓存）。
2. Gate B 明确注入
   `SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f443a-cheap-combined` 与任务指定的
   `CARGO_TARGET_DIR`，或等价地修正 gate 命令，使 receipt test 不再隐式选择 main。
3. 只重验 Gate A 的三个被遮挡 Rust target、Gate B receipt test，以及 Gate C 的两个
   isolated service graph；已经绿色的 checker/corpus/Router/list/receipt probes
   无需在本节点内反复完整重跑。

本节点没有启动 stable、watch、MongoDB、固定端口、外部 network 或 live；没有修改
代码、fixture、manifest、lockfile或其它 result；没有 merge、rebase 或 push。
