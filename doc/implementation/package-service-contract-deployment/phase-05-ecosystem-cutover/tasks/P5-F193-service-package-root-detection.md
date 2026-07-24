# P5-F193：Service Package 根目录识别

状态：Ready

## 直接父任务

- `P5-F180L-actor-full-chain-acceptance-result.md`

## 问题与目标

`scripts/skiff.mjs detectRootKind()` 当前把同时含 `package.yml + service.yml` 的目录判为 ambiguous；
但目标模型中 service 首先是 Package，`service.yml` 是 Package 的附加 service role。修改 CLI/test/
authoring 根目录识别，使 package manifest 决定根身份，service manifest 在其上启用服务发布与测试。

不得要求 Registry 删除任一 manifest，也不得恢复独立 service source root。

## 验证

- package-only；
- package + service；
- legacy service-only 的明确错误或迁移行为；
- 多/缺 manifest；
- `skiff test registry` 到达真实 authoring；
- CLI 聚焦测试、diff check；
- 独立提交和 result。

