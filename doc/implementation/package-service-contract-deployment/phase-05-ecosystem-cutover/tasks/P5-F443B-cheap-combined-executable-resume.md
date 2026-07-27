# P5-F443B Cheap combined executable resume

状态：Ready。只读gate恢复；只重验F443A因磁盘和错误root被遮挡的范围。

## 直接父节点

- `P5-F443A-phase05-cheap-combined-integration-result.md`

F443A已经在相同production tree上通过：

- source checker、三组cross-system verifier；
- Router type-check与8文件169/169；
- Rust transport/request/eval三个WebSocket JSON-RPC target；
- skiff-packages type-check/list与Internals四份receipt tests。

失败不是候选断言：shared target所在volume满，以及Gate B漏传`SKIFF_ROOT`。环境owner已只清理
精确shared Cargo target，释放54.6GB；源码、候选、stable均未改。

## 精确候选

| Repo | Root | Expected commit |
| --- | --- | --- |
| Skiff integration | `/Users/geek/workspace/skiff-phase-05-integration` | `7d6f4de8` |
| Internals | `/Users/geek/workspace/internals-phase-05-integration` | `2320949` |
| skiff-packages | `/Users/geek/workspace/skiff-packages-phase-05-integration` | `19cfab5d` |

本gate专用Skiff worktree相对integration只增加调度文档；production tree必须bit-identical。

## 只重验的Gate A

使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

运行：

```bash
cargo test -p skiff-runtime-host websocket_jsonrpc --no-fail-fast
cargo test -p skiff-runtime-package-test --test package_artifact \
  entrypoint_validation_rejects_non_exact_gateway_facts
cargo test -p skiff-test-runner --test package_service_contract_deployment
```

不重跑F443A已绿的checker/corpus/Router与前三个Rust target。

## 修正后的Gate B

在`/Users/geek/workspace/skiff-packages-phase-05-integration`执行：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f443b-cheap-combined-resume \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  node --test \
  scripts/registry-service-source.test.mjs \
  scripts/registry-service-receipt.test.mjs
```

必须证明receipt workflow实际使用candidate中声明
`skiff-package-service-smoke-fixture`的test-runner，不能回退主仓库。

## 恢复Gate C

在Internals分别从`aihub/service`与`agine/service`执行：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f443b-cheap-combined-resume \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  npm run type-check
```

两个workflow必须使用隔离临时artifact root；不得触碰stable watch/artifact root。

## 结论与边界

结束时记录：

- 所有命令test/count；
- 三仓库HEAD/tree/status；
- free space；
-临时目录/symlink清理状态。

结论只能是：

- `PASS / STABLE_CANDIDATE_READY`
- `COMBINED_BLOCKED`，精确列候选断言失败与最小owner
- `GATE_NOT_EXECUTABLE`，只用于再次出现环境/命令阻塞

不得修复、不得启动stable/network/live、不得merge/rebase/push、不得派子Agent。

worktree：

`/Users/geek/workspace/skiff-p5-f443b-cheap-combined-resume`

branch：

`codex/p5-f443b-cheap-combined-resume`

只新增并提交：

`P5-F443B-cheap-combined-executable-resume-result.md`
