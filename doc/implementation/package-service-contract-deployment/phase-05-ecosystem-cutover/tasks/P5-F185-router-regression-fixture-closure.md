# P5-F185：Router 回归夹具收敛

状态：Ready

## 直接父任务

- `P5-F180L-actor-full-chain-acceptance-result.md`

## 目标

修复 F180L 后 Router 全量测试仅剩的旧 compiler authoring fixture 与时间敏感 spawn queue 测试，使
Router 全量测试在当前 canonical compiler/loader/runtime 协议上稳定通过。

## 必须实现

- 旧 fixture 使用当前 compiler CLI 和 v3 artifact/contract/protocol，不恢复已删除 CLI；
- fixture 必须由真实 authoring 产物生成，不能手写绕过 identity/loader 校验；
- spawn queue 测试使用可注入时钟或事件同步，不依赖固定日期或脆弱 sleep；
- 不修改生产超时语义来迁就测试。

## 验证

- Router 全量测试通过；
- Router type-check；
- `git diff --check`；
- 独立提交并写 result。

