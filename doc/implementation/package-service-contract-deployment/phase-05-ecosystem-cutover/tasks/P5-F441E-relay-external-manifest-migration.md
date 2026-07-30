# P5-F441E codex-relay external manifest migration

状态：Ready。对应 F440A 的 IC2；确定性 Internals leaf。

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

- `codex-relay/service/service.yml`
- `codex-relay/service/http.yml`
- `codex-relay/service/config.dev.yml`
- `codex-relay/service/service-api-receipt.test.mjs`

Skiff侧只可新增本 leaf result。不得修改AIHub、Agine、Account、shared workflow、Skiff production或其它
task/result。不得派子 agent。

要求：

- service只保留id与`serviceCalls: [relayProxy]`；
- 30个HTTP entry逐项搬到`http.yml`，保持27 unary / 3 server-stream；
- config.dev timeout owner保持120000；
- receipt直接读`http.yml`，不得继续从service字符串切片；
- 改前后PackageBuildId与ServiceProtocolIdentity exact相等；
- 新record为30 gateway/ingress，Gateway v2、DeploymentArtifact v3；
- relay对上游HTTP stream的raw格式及handler不在本leaf改变。

先保存baseline receipt。必跑：

```bash
node --test codex-relay/service/service-api-receipt.test.mjs
node --check codex-relay/service/service-api-receipt.test.mjs
npm run type-check
git diff --check
```

使用指定Skiff toolchain，不访问stable/live；跨service type-check遮挡只记录，不越界。

交付：

- implementation worktree：`/Users/geek/workspace/internals-p5-f441e-relay-manifest`
- branch：`codex/p5-f441e-relay-manifest`
- result worktree：`/Users/geek/workspace/skiff-p5-f441e-relay-manifest-result`
- result：`P5-F441E-relay-external-manifest-migration-result.md`

两仓分别提交；不 merge/rebase/push。
