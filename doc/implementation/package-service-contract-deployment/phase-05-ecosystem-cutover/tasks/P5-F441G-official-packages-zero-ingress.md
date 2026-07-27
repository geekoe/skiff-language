# P5-F441G Official packages zero-ingress verification

状态：Ready。对应 F440A 的 P0；确定性 skiff-packages leaf。

## 直接父节点

- `P5-F440A-external-manifest-owner-audit-result.md`
- `P5-F440H-external-manifest-strict-dto-compiler-checkpoint-result.md`
- `P5-F440M-external-manifest-identity-deployment-follower-result.md`

输入：

- skiff-packages：`f8c634ce4573506e35f6bc1c7cc1e4eef9992a78`
  / tree `eb00877ef260d122552af1ff0491c74102adbd57`
- Skiff：`67d61b8db9cb1750fe624dc40b9968642fb6d7f3`
  / tree `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff`

## 目标

证明official packages的7个service root在strict split-manifest hard cut后仍是canonical zero-ingress：

- `registry`
- `tests/aliyunoss`
- `tests/http-session`
- `tests/openai-live`
- `tests/openai`
- `tests/registry`
- `tests/track`

不得为了“迁移”伪造空`http.yml`或`websocket.yml`。service只含id、可选kind、可选serviceCalls；timeout继续
由现有config profile拥有。若production manifest无需内容diff，允许实现提交只改测试/receipt；不得制造
无语义改动。

## 唯一写集

- `registry/service.yml`
- `registry/config.dev.yml`
- `tests/*/service.yml`
- `tests/*/config.*.yml`
- `scripts/registry-service-source.test.mjs`
- `scripts/registry-service-receipt.test.mjs`
- `scripts/test-packages.mjs`
- Skiff侧本 leaf result

禁止修改package源码/API、Skiff production、其它repo或其它task/result。不得派子 agent。

## 验证

先保存Registry baseline receipt，再用指定Skiff toolchain重新build：

```bash
node --test scripts/registry-service-source.test.mjs scripts/registry-service-receipt.test.mjs
node scripts/test-packages.mjs
node --check scripts/test-packages.mjs
git diff --check
```

若`test-packages.mjs`包含外部账号/live测试，只运行其fixture-only/list/dry-run或明确的离线selector；不得访问
外部服务或stable/live。

必须证明：

- `find registry tests -name http.yml -o -name websocket.yml`为空；
- Registry gatewayEntries/ingress均为0；
- 20个serviceCalls与exact operation closure保持；
- 当前Gateway v2 / DeploymentArtifact v3 schema被receipt消费；
- package/contract identity是否保持由真实receipt给出，不手写猜测。

## 交付

- implementation worktree：`/Users/geek/workspace/skiff-packages-p5-f441g-zero-ingress`
- branch：`codex/p5-f441g-zero-ingress`
- result worktree：`/Users/geek/workspace/skiff-p5-f441g-official-packages-result`
- result：`P5-F441G-official-packages-zero-ingress-result.md`

两仓分别提交；不 merge/rebase/push。
