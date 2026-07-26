# P5-F420D Remove obsolete tar oracle and final N4 gate

状态：Ready（F420C scope-expansion 后继）。

## 直接父节点

- `P5-F420C-tooling-fixture-and-format-gate-result.md`

F420C 已用真实current package publish证明：满足artifact root、compiler input与toolchain前置后，
publish直接成功；全仓production不存在`tar`调用或`failed to spawn tar`错误owner。原
missing-tar test已经失去被测行为。当前同一test file另有
`runtime and compiler DAG adapters promote spawn failure before status interpretation`，它已真实覆盖
current missing-cargo safe-outcome；把tar test改名复制成missing-cargo只会重复覆盖。

因此本节点删除失去production owner的obsolete tar oracle，并在最终exact tree上完成全部N4门禁。

## 精确起点与范围

- integrated start：
  `9ed8c2bcd2918d4f1b60ab5c5ceeefa6519d68eb`；
- tree：
  `192f892b6744a8515443fb52303053398495c1b7`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明start/tree与F415 ancestry。唯一允许写入：

```text
scripts/tests/command-caller-migrations.test.mjs
本任务 result
```

不得修改production、其它test、验证计划、锁文件或生态仓库；不得派子Agent、merge/rebase/push、
访问stable/live、instance或watch registry。

## 必须修改

1. 删除整个`missing tar is reported through the safe outcome failure before remote I/O` test；
2. 删除随之成为unused的fixture/constant/import，但不能改其余三个test语义；
3. direct listing/execution必须从4项变成精确3项，三项全部执行通过；
4. production与test中`failed to spawn tar` / tar command oracle反搜为0；
5. 保留current missing-cargo DAG test、instance missing lsof/ps test、真实ownership receipt下
   exit 9与invalid JSON test。

## 最终N4门禁

所有Cargo命令使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

先listing再execution，运行：

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

只有全部通过且`router/**`、production与F415 mapping tree相对父节点零diff，才判定N4 `PASS`并解除
F421。

## 交付

实现与`P5-F420D-remove-obsolete-tar-oracle-and-final-gate-result.md`分开提交。result记录删除依据、
direct 3/3、tooling完整计数、Router 608/608、Node/test-runner/source-suite/identity结果、格式/diff、
production零owner反搜与F421是否解除。保持clean；不merge/rebase/push。

任何新production修改需要或范围外失败都返回`TASK_SCOPE_EXPANDED`。
