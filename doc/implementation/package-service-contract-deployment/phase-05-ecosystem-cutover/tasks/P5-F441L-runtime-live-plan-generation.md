# P5-F441L Runtime-live plan generation

状态：Ready。只改runtime-live计划与direct plan tests，不运行live workload。

## 直接父节点

- `P5-F441J-live-harness-execution-preflight-result.md`
- `P5-F441I-canonical-live-source-root-authoring-result.md`
- `P5-F441H-test-service-profile-target-environment-separation-result.md`
- `P5-F441A-external-control-file-discovery-result.md`

父节点已冻结canonical root、固定`skiff-test` profile、caller-owned target environment、无base assembly与
逐activation generation推进。引用链继续追溯到唯一权威设计；本leaf不得重新解释profile或test execution
语义。

实现基线为`83543c1cd21bbb454750cbf5ee6e1d51ada987f0`。

## DAG位置与目标

本节点是F441J解除的runtime-live plan consumer。完成后，`runtime-live` selector必须：

1. 只发现canonical `runtime/live-tests` package root下的四个tracked `.live.test.skiff`；
2. 为每个fixture生成current runner参数，显式携带artifact root、唯一absolute platform source root、
   activation URL、ingress URL、target environment、expected generation、`--deny-skips`与
   `--require-tests`；
3. 不生成`--base-assembly`；
4. 按canonical fixture排序，从caller提供的`N`依次生成`N`、`N+1`、`N+2`、`N+3`；
5. 对非法或无法精确递增的generation fail closed，不发生JS精度截断；
6. 把F441I后的真实canonical roots视为positive plan输入，删除只因legacy root形状成立的过期断言。

本节点只形成structural executable-plan检查点。F441I已记录的`__skiffPayload`与
expected-platform-error属于后继test-runner execution owner；本leaf不得skip/delete case或放宽canonical
policy来掩盖它们。

## 唯一写集

- `scripts/lib/verify-live-plan.mjs`
- `scripts/tests/verify.test.mjs`
- `scripts/tests/verify-live-registry.test.mjs`
- `scripts/tests/verify-live-plan-platform-source.test.mjs`
- 可新增一个只测试runtime generation的
  `scripts/tests/verify-live-plan-runtime-generation.test.mjs`
- 本leaf result

禁止修改encrypted harness、live source roots、test-runner、live registry production定义、Compiler、
Router/Runtime production、其它fixture/task/result或stable/live状态。不得派子Agent。

## Current参数与generation

每个phase的runner参数终态为：

```text
cargo run --manifest-path test-runner/Cargo.toml -- \
  <fixture> \
  --live \
  --artifact-root <canonical store> \
  --platform-source-root <absolute repo root> \
  --activation-url <canonical activation URL> \
  --ingress-url <canonical ingress origin> \
  --environment <caller-owned target environment> \
  --expected-generation <N + fixture index> \
  --deny-skips \
  --require-tests
```

现有command/bin形式若已由registry冻结则保持，不为格式偏好改写。关键是传给runner的参数集合与值精确；
target environment不能用于选择或验证`config.<environment>.yml`，plan只要求source root拥有固定
`config.skiff-test.yml`。

fixture discovery/order必须可复现；generation增量与该精确顺序绑定。若最终值超过current canonical
generation范围，plan构造必须在任何phase执行前终止并给出明确诊断。

## 测试先行与验证

先新增多fixture generation断言，使旧实现因重复使用同一个generation失败，再实现。至少覆盖：

- 单fixture保留`N`；
- 四fixture得到连续且唯一的`N..N+3`；
- 大整数不发生Number精度损失，越界/非法输入fail closed；
- 每个phase只有一个absolute`--platform-source-root`；
- 四个phase均无`--base-assembly`且保留deny/require flags；
- current canonical root为positive；缺package owner、缺artifact root、非法URL/target/generation仍fail
  closed；
- execution preflight在TOCTOU后仍拒绝已消失fixture/root。

只运行non-live验证：

```bash
node --test \
  scripts/tests/verify.test.mjs \
  scripts/tests/verify-live-registry.test.mjs \
  scripts/tests/verify-live-plan-platform-source.test.mjs \
  scripts/tests/verify-live-plan-runtime-generation.test.mjs
node --check scripts/lib/verify-live-plan.mjs
git diff --check
```

若未新增generation专用文件，从命令中删除不存在的路径，并在result记录实际test文件/count；不得用零测试
命令充当证据。

## 停止与交付

若generation推进需要修改registry schema、test-runner或activation wire，返回
`TASK_SCOPE_EXPANDED`并给出精确证据；不得越界。发现execution blocker不应吞入本leaf。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f441l-runtime-live-plan`
- branch：`codex/p5-f441l-runtime-live-plan`
- result：`P5-F441L-runtime-live-plan-generation-result.md`

Implementation与result分开提交；不merge/rebase/push。
