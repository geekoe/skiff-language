# P5-F200：可选 Deployment Timeout Policy 结果

状态：Done

## 直接父任务

- `P5-F200-optional-deployment-timeout-policy.md`

## 结果

- `DeploymentPolicy.timeoutMs`收敛为可选override：
  - profile缺省或`null`生成`None`；
  - canonical JSON省略`timeoutMs`，不填虚假默认值；
  - 显式正整数毫秒值生成`timeoutMs`；
  - 显式零、负数、小数、字符串和对象形状全部fail closed。
- artifact反序列化同时接受缺省与`null`，两者归一成同一个typed policy并产生相同
  `DeploymentArtifactIdentity`。
- `policy.timeoutMs = 0`仍由artifact validation拒绝；缺省不再被误判为零。
- compiler input测试使用当前Account `config.dev.yml`的真实profile内容复现未声明timeout时的
  canonical `null`，并锁定profile额外字段拒绝行为。
- 所有现有显式timeout fixture都改为`Some(...)`，没有改变其既有策略和identity覆盖范围。
- 权威架构文档已明确optional timeout语义。

## 验证

- `cargo test -p skiff-compiler --test generated_service_deployment`
  - 8 passed。
- `cargo test -p skiff-compiler-input service_config`
  - 7 passed。
- `cargo test -p skiff-artifact-model`
  - 114 passed。
- `cargo test -p skiff-artifact-identity`
  - 87 passed，另有1个既有ignored regeneration test。
- `cargo test -p skiff-deployment`
  - 52 passed。
- `cargo check --workspace --tests`
  - 通过。
- `git diff --check`
  - 通过。

`scripts/check-artifact-identity-single-source.mjs`中的DTO probe已同步为`Option<u64>`；该全仓
checker仍报告当前integration HEAD已有的六项无关基线缺口，本任务没有扩展或隐藏这些缺口。
没有启动或修改stable instance，没有push。
