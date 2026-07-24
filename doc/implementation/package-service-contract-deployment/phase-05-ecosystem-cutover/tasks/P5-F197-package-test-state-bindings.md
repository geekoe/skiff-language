# P5-F197：Package Test Database State Bindings

状态：Ready

## 直接父任务

- `P5-F190-database-state-requirement-projection-result.md`

## 目标

package-test overlay/ServiceDeployment 必须为 PackageArtifact 声明的每个 exact database state
requirement 生成测试自有 typed binding 和隔离 namespace。当前 http-session/track 测试因
`missing state binding ...` 失败。

不得删除 package state 声明、使用 ambient/stable DB 或按未声明名字猜测。

## 验证

- http-session、track 真实 `skiff test`；
- 多 state、缺失、额外、错误 kind；
- 不同测试运行 namespace 隔离；
- test-runner/deployment/runtime package-test 聚焦测试；
- workspace check、diff check、独立提交和 result。

