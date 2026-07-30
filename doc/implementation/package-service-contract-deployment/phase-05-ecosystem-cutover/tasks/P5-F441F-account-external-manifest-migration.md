# P5-F441F Account external manifest migration

状态：Ready。对应 F440A 的 IK2；确定性 Internals leaf。

## 直接父节点

- `P5-F440A-external-manifest-owner-audit-result.md`
- `P5-F440H-external-manifest-strict-dto-compiler-checkpoint-result.md`
- `P5-F440M-external-manifest-identity-deployment-follower-result.md`

输入：

- Internals：`8ccc6cc5a066e674964c3b88e86316d67adfcb1a`
  / tree `817591e145395bc514538a0480decc4e5be9f1f0`
- Skiff：`67d61b8db9cb1750fe624dc40b9968642fb6d7f3`
  / tree `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff`

## 目标与写集

唯一 implementation写集：

- `skiff-platform/account/service.yml`
- `skiff-platform/account/http.yml`
- `skiff-platform/account/config.dev.yml`
- `skiff-platform/account/service-api-receipt.mjs`
- `skiff-platform/account/service-api-receipt.test.mjs`

Skiff侧只可新增本 leaf result。不得修改AIHub、Agine、Relay、shared workflow、Skiff production或其它
task/result。不得派子 agent。

要求：

- service只保留id；
- 21个raw HTTP entry逐项搬到顶层map `http.yml`；
- timeout只由既有`config.dev.yml:120000`拥有；
- receipt直接读`http.yml`，删掉依赖`lines[1] == "http:"`的旧解析；
- PackageBuildId和zero-operation ServiceContract/Protocol identity改前后exact相等；
- 新deployment为21 gateways/ingress、Gateway v2、DeploymentArtifact v3。

先保存baseline receipt。必跑：

```bash
node --test skiff-platform/account/service-api-receipt.test.mjs
node --check skiff-platform/account/service-api-receipt.mjs
node --check skiff-platform/account/service-api-receipt.test.mjs
npm run type-check
git diff --check
```

使用指定Skiff toolchain，不访问stable/live；跨service type-check遮挡只记录，不越界。

交付：

- implementation worktree：`/Users/geek/workspace/internals-p5-f441f-account-manifest`
- branch：`codex/p5-f441f-account-manifest`
- result worktree：`/Users/geek/workspace/skiff-p5-f441f-account-manifest-result`
- result：`P5-F441F-account-external-manifest-migration-result.md`

两仓分别提交；不 merge/rebase/push。
