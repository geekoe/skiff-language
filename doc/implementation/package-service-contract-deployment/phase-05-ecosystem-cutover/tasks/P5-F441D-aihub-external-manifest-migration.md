# P5-F441D AIHub external manifest migration

状态：Ready。对应 F440A 的 IA2；确定性 Internals leaf。

## 直接父节点

- `P5-F440A-external-manifest-owner-audit-result.md`
- `P5-F440H-external-manifest-strict-dto-compiler-checkpoint-result.md`
- `P5-F440M-external-manifest-identity-deployment-follower-result.md`

输入：

- Internals：`8ccc6cc5a066e674964c3b88e86316d67adfcb1a`
  / tree `817591e145395bc514538a0480decc4e5be9f1f0`
- Skiff toolchain：`67d61b8db9cb1750fe624dc40b9968642fb6d7f3`
  / tree `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff`

## 目标与写集

唯一 implementation写集：

- `aihub/service/service.yml`
- `aihub/service/http.yml`
- `aihub/service/config.dev.yml`
- `aihub/service/service-api-receipt.mjs`
- `aihub/service/service-api-receipt.test.mjs`

Skiff侧只可新增本 leaf result。不得修改Agine、Relay、Account、shared workflow、Skiff production或其它
task/result。不得派子 agent。

迁移要求：

- service只保留id和现有2个`serviceCalls`；
- 7个原HTTP entry逐项原样搬到顶层map `http.yml`；
- 5 unary / 2 server-stream、handler、selector与adapter配置精确保持；
- 删除service中的重复timeout owner，只保留既有`config.dev.yml timeout:120000`；
- receipt改读独立文件，不保留字符串slice兼容；
- 改前后PackageBuildId与ServiceProtocolIdentity必须exact相等；
- 新deployment使用v3、gateway identity使用v2，gateway count=7。

先保存旧canonical receipt，再改文件。必跑：

```bash
node --test aihub/service/service-api-receipt.test.mjs
node --check aihub/service/service-api-receipt.mjs
node --check aihub/service/service-api-receipt.test.mjs
npm run type-check
git diff --check
```

构建/receipt使用指定Skiff toolchain，不访问stable/live。若shared Internals type-check被其它service旧manifest
遮挡，记录精确首错并继续完成AIHub direct receipt，禁止改其它service。

交付：

- implementation worktree：`/Users/geek/workspace/internals-p5-f441d-aihub-manifest`
- branch：`codex/p5-f441d-aihub-manifest`
- result worktree：`/Users/geek/workspace/skiff-p5-f441d-aihub-manifest-result`
- result：`P5-F441D-aihub-external-manifest-migration-result.md`

两仓分别提交；不 merge/rebase/push。
