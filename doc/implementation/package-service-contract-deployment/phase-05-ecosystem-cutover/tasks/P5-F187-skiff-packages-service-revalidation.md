# P5-F187：官方 Packages 与 Registry 服务复验

状态：Ready

## 直接父任务

- `P5-F180L-actor-full-chain-acceptance-result.md`

## 目标

使用当前 Skiff integration 对 `skiff-packages` integration 中的官方 packages、Registry service 和
真实 package/service authoring 重新编译、测试并修复，不使用 stable instance。

## 必须实现

- 运行仓库既有 package 与 Registry service 测试；
- 真实生成 PackageArtifact、ServiceContract、Deployment/Assembly；
- std 与普通 Package schema 使用同一 canonical store；
- 不使用 artifact rewrite、旧 boundary schema 或手写兼容层；
- 仅修改 `skiff-packages` 任务 worktree，独立提交和 result。

## 验证

- 官方 package/Registry 测试全通过；
- `node skiff/scripts/skiff.mjs test` 对相关真实路径通过；
- 工作区 diff check。

