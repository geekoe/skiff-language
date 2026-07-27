# P5-F441C External manifest live-root and harness migration

状态：Ready。对应 F440A 冻结 DAG 的 S2；只做非live实现与验证。

## 直接父节点

- `P5-F440A-external-manifest-owner-audit-result.md`
- `P5-F440H-external-manifest-strict-dto-compiler-checkpoint-result.md`
- `P5-F440M-external-manifest-identity-deployment-follower-result.md`

实现基线为 `67d61b8db9cb1750fe624dc40b9968642fb6d7f3`
（tree `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff`）。

F441A是并行sibling及最终classifier验证前置，不是本任务的设计输入。F441A未合流时，只能在自己写集内
完成迁移并记录classifier验证遮挡；不得读取其未完成结果或复制其改动。

## 目标

把三个Skiff live source root一次迁到canonical package/service布局，但不运行managed live：

- `runtime/encrypted-storage-live/default-service`
- `runtime/encrypted-storage-live/mapped-service`
- `runtime/live-tests`

每个root必须有canonical `package.yml`、`api.yml`、精简`service.yml`、独立`http.yml`与实际被harness选择的
tracked `config.<profile>.yml`。旧`packages`移回package owner；旧routes转为named `rawHttp` entries；
global guard逐entry复制并去除废弃`root.` selector；timeout只在profile中。

## 唯一写集

- `runtime/encrypted-storage-live/default-service/**`
- `runtime/encrypted-storage-live/mapped-service/**`
- `runtime/live-tests/**`
- `scripts/lib/encrypted-storage-live-harness.mjs`
- `scripts/check-db-encrypted-storage-live.mjs`
- `scripts/lib/verify-live-plan.mjs`
- 上述harness/checker的直接verify tests：
  - `scripts/tests/verify.test.mjs`
  - `scripts/tests/verify-live-registry.test.mjs`
  - `scripts/tests/verify-live-plan-platform-source.test.mjs`
- 本 leaf result

禁止修改classifier/parser、stable instance、其它fixtures、Router/Runtime production、其它task/result。
不得派子 agent。

## 精确迁移

- default-service：21个`rawHttp`，guard保留，`config.dev.yml timeout:120000`。
- mapped-service：13个`rawHttp`，guard保留，旧package dependencies移入`package.yml`，
  `config.dev.yml timeout:120000`。
- runtime/live-tests：6个`rawHttp`，其中5 unary、1 server-stream；timeout写入harness真实选择的tracked
  profile。

先只读证明每个root当前由哪个命令/profile/role消费。若`kind`或runtime-live profile不能从现有调用链唯一
确定，不得猜测：停止并在result列出候选、调用点和需要用户决定的唯一问题。

不得新增dual format、alias或临时兼容。canonical receipt应为总计40个HTTP ingress。

## 测试与验证

先更新direct plan/check tests使旧root失败，再迁移。只运行non-live验证：

```bash
node --test scripts/tests/verify.test.mjs \
  scripts/tests/verify-live-registry.test.mjs \
  scripts/tests/verify-live-plan-platform-source.test.mjs
node scripts/check-db-encrypted-storage-live.mjs --help
node --check scripts/lib/encrypted-storage-live-harness.mjs
node --check scripts/lib/verify-live-plan.mjs
git diff --check
```

如现有checker提供明确的plan-only/dry-run入口，运行它；禁止启动Mongo、Router、Runtime、telemetry、
instance、watch、stable或任何live workload。

反向搜索三个root：

```bash
rg -n '^[[:space:]]*(version|packages|routes|timeout):|root\\.' \
  runtime/encrypted-storage-live/default-service \
  runtime/encrypted-storage-live/mapped-service runtime/live-tests
```

result列出每root的entry count、U/S、guard、profile、PackageBuild/ABI是否保持及新deployment/assembly receipt；
若F441A尚未合流导致classifier gate失败，明确标记遮挡，不能改F441A文件。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f441c-external-live-roots`
- branch：`codex/p5-f441c-external-live-roots`
- result：`P5-F441C-external-live-root-migration-result.md`

Implementation 与 result 分开提交；不 merge/rebase/push。
