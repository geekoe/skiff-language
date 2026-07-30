# P5-F420C Tooling fixture and format gate

状态：Ready（F420B scope-expansion 后继）。

## 直接父节点

- `P5-F420B-retire-runtime-assembly-websocket-residue-result.md`

F420B 已在 exact candidate 上完成 Router 终态并通过 608/608、TypeScript、Node、test-runner、
source-suite 与 Router verify。剩余三个失败都是候选零 diff 的旧测试前置/格式漂移：

1. missing-tar fixture没有提供 current package publish 必需的 `--artifact-root`；
2. isolated status fixture没有先建立 current workspace ownership receipt；
3. F420 test-runner checkpoint有一处 rustfmt drift。

本节点只修这三个测试/格式 owner并重跑最终 N4 gate；不修改 production。

## 精确起点与范围

- integrated start：
  `273d9309c0650bad75fa08c88684359995711b91`；
- tree：
  `7b860e6b026e7666c1279a3118765ddd7ff21979`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明 start/tree与F415 ancestry。唯一允许写入：

```text
scripts/tests/command-caller-migrations.test.mjs
test-runner/tests/package_service_contract_deployment.rs  # 仅rustfmt
本任务 result
```

禁止修改 production、其它 test、Router、锁文件、生态仓库或验证计划。不得派子 Agent、
merge/rebase/push、访问 stable/live、instance 或 watch registry。

## 必须修复

### Missing tar fixture

- 在 fixture 自有临时目录内创建独立 artifact root；
- package publish命令显式传且只传一次 `--artifact-root <path>`；
- 仍通过空 `PATH` 实际到达 missing `tar`，断言安全错误中有
  `failed to spawn tar: ENOENT`，且不泄漏 `spawnargs` / `cause`；
- 不放宽 CLI 对缺失 `--artifact-root` 的 production 校验。

### Isolated status fixture

- 使用 current `claimIsolatedTestWorkspace` 建立真实 ownership marker/receipt；
- 在该owned root内建立config，再用 `captureIsolatedTestConfig` 得到要求config的receipt；
- 两次 `verifyInstanceStopped` 都传该合法receipt，使第一项真实到达 fake child exit 9，第二项真实
  到达 invalid JSON；
- 不伪造 dev/ino/nonce，不跳过 production ownership验证。

### rustfmt

只接受 rustfmt 对
`test-runner/tests/package_service_contract_deployment.rs` 已有 marker lookup 的机械排版；
语义、断言与测试数量不变。若 `cargo fmt` 要修改其它文件，停止并上报，不得扩大。

## 验证与N4终判

所有 Cargo命令使用共享 target：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

先 listing再execution，至少运行：

```bash
node --test scripts/tests/command-caller-migrations.test.mjs
node scripts/verify.mjs --only tooling

node --test \
  scripts/tests/artifact-identity-validation.test.mjs \
  scripts/tests/package-service-authoring.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs \
  scripts/tests/runtime-execution-boundary-checker.test.mjs \
  scripts/tests/skiff-source-test-suite.test.mjs
node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/check-artifact-identity-single-source.mjs

node scripts/verify.mjs --only router

cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --list
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --test-threads=1
node scripts/run-skiff-tests.mjs

cargo fmt --all -- --check
git diff --check
```

只有上述全部通过，且F420B的Router production/tree没有被修改，才把 N4 判为 `PASS` 并解除 F421。

## 交付

实现与 `P5-F420C-tooling-fixture-and-format-gate-result.md` 分开提交。result记录 exact
commit/tree、两个失败优先级如何恢复、ownership receipt事实、格式零语义变化、所有命令实际计数、
继承并复验的Router 608/608，以及F421是否解除。保持 clean；不 merge/rebase/push。

任何 production 修改需要或新范围外失败都返回 `TASK_SCOPE_EXPANDED`。
