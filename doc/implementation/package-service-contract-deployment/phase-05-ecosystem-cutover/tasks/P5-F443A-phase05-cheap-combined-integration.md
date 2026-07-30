# P5-F443A Phase 5 cheap combined integration

状态：Ready。只读gate owner；在三仓库当前integration候选上验证共享接线，决定能否冻结稳定候选。

## 直接父节点

- `P5-F440Z3E-router-websocket-rpc-gateway-integration-resume-result.md`
- `P5-F442B-rust-test-runner-fixture-closeout-result.md`
- `P5-F442C-cross-system-corpus-verifier-closeout-result.md`
- `P5-F442D-source-layout-checker-closeout-result.md`
- `P5-F441G-official-packages-zero-ingress-result.md`

父节点已分别通过局部验证。本节点不重复完整gate，而是在合流后的精确代码状态上覆盖
compiler/artifact → Router/Runtime/Host → test-runner → official/internals service authoring共同入口。

## 精确候选

| Repo | Root | Branch | Start commit |
| --- | --- | --- | --- |
| Skiff production tree | `/Users/geek/workspace/skiff-phase-05-integration` | `codex/package-service-phase-05` | `acbf0ab0` |
| Internals | `/Users/geek/workspace/internals-phase-05-integration` | `codex/package-service-phase-05` | `2320949` |
| skiff-packages | `/Users/geek/workspace/skiff-packages-phase-05-integration` | `codex/package-service-phase-05` | `19cfab5d` |

Gate在本任务专用Skiff worktree执行；它相对`acbf0ab0`只多本调度文档，production tree必须
bit-identical。开始与结束都必须确认三个候选worktree clean且production commit/tree不变。
只允许新增本节点result；不得修改代码、fixture、manifest、lockfile或其它result。

## Gate A：Skiff共同接线

### Node/checker/corpus

```bash
node scripts/check-skiff-source-layout.mjs
node cross-system-fixtures/package-service-ecosystem/verify.mjs --self-test
node cross-system-fixtures/package-service-ecosystem/verify.mjs --combined-probe
node cross-system-fixtures/package-service-ecosystem/verify.mjs \
  --runtime-wire-self-test
pnpm --dir router type-check
```

Router先list并记录non-zero count，再run：

```bash
router/node_modules/.bin/vitest list --root router \
  tests/websocket-jsonrpc-gateway.test.ts \
  tests/websocket-rpc-bridge.test.ts \
  tests/websocket-request-broker.test.ts \
  tests/runtime-assembly-websocket-rpc-snapshot.test.ts \
  tests/runtime-endpoint-source-lifecycle.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts \
  tests/runtime-assembly-request-wire.test.ts \
  tests/compilerGeneratedManifestCompatibility.test.ts
router/node_modules/.bin/vitest run --root router \
  tests/websocket-jsonrpc-gateway.test.ts \
  tests/websocket-rpc-bridge.test.ts \
  tests/websocket-request-broker.test.ts \
  tests/runtime-assembly-websocket-rpc-snapshot.test.ts \
  tests/runtime-endpoint-source-lifecycle.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts \
  tests/runtime-assembly-request-wire.test.ts \
  tests/compilerGeneratedManifestCompatibility.test.ts
```

### Rust shared target

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-transport runtime_assembly_websocket_jsonrpc
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-request websocket_jsonrpc_execution
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-eval runtime_websocket_jsonrpc
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-host websocket_jsonrpc --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-package-test --test package_artifact \
  entrypoint_validation_rejects_non_exact_gateway_facts
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-test-runner --test package_service_contract_deployment
```

## Gate B：skiff-packages

在`/Users/geek/workspace/skiff-packages-phase-05-integration`：

```bash
npm run type-check
node --test \
  scripts/registry-service-source.test.mjs \
  scripts/registry-service-receipt.test.mjs
node scripts/test-packages.mjs --list
```

只list package tests，不执行external/live。

## Gate C：Internals current authoring

在`/Users/geek/workspace/internals-phase-05-integration`：

```bash
node --test \
  agine/service/service-api-receipt.test.mjs \
  aihub/service/service-api-receipt.test.mjs \
  codex-relay/service/service-api-receipt.test.mjs \
  skiff-platform/account/service-api-receipt.test.mjs
```

然后用current integration toolchain与official package root运行两个共享service graph：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f443a-cheap-combined \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  npm run type-check
```

分别从：

- `aihub/service`
- `agine/service`

执行。它们只能使用隔离临时artifact root，不得读取或写入stable artifact/watch registry。

## 结束检查与结论

```bash
git diff --check
git status --short
```

三个repo分别执行。临时dependency symlink或可再生成缓存必须在result提交前清理。

结论只能是：

- `PASS / STABLE_CANDIDATE_READY`：所有规定命令绿色、三仓库候选不变且clean；
- `COMBINED_BLOCKED`：列每个独立gate的最早真实失败、遮挡范围、最小owner与建议修复节点；
- `GATE_NOT_EXECUTABLE`：只用于依赖/命令本身无法运行，必须给等价探针或精确缺口。

一个gate失败后仍运行彼此独立、成本合理的其它gate一次，以便批量分类；不得修复、不得反复完整重跑。

## 操作边界与交付

- 不启动stable、watch、MongoDB、固定端口、外部network或live；
- 不merge/rebase/push；
- 不派子Agent；
- 不修改候选。

worktree：

`/Users/geek/workspace/skiff-p5-f443a-cheap-combined`

branch：

`codex/p5-f443a-cheap-combined`

只新增：

`P5-F443A-phase05-cheap-combined-integration-result.md`

只提交result；不merge/rebase/push。
