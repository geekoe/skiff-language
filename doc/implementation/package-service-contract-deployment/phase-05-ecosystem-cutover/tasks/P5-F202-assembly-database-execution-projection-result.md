# P5-F202 Assembly database execution projection result

状态：Completed

## 结果

canonical assembly 数据库执行不再构造或依赖 legacy `RuntimeProgram`。

- 普通数据库命令的结果类型规划和命令执行使用
  `RuntimeExecutionProjection::for_context`；该部分由前置提交 `e01ae61` 落地。
- 数据库可恢复值路径改用
  `EvalRecoverableBehaviorHooks::new_for_execution`，直接索引当前 assembly execution
  image；legacy execution 仍通过同一枚举的 `Legacy` 分支运行。
- assembly 的直接 `Address` 类型引用在生成运行时类型计划前先转换为 canonical
  assembly type address。合法的 `FileIrIdentity` 地址因此解析成实际声明；不存在的
  type index 精确返回 `TypeIndexOutOfBounds`，不再静默降级为 `Unknown`。
- 未增加 assembly-to-legacy adapter、默认数据库、全局 namespace、动态类型 fallback
  或文件系统查找。F197 的精确 state binding/namespace 消费未改动。

## 测试

- `cargo test -p skiff-runtime-eval`：120/120 通过。
  - assembly 普通数据库结果类型从 execution image 解析；
  - assembly 可恢复数据库 behavior hooks 从 execution image 建立；
  - 缺失 assembly type index 失败关闭；
  - 既有 legacy 数据库与可恢复测试保持通过。
- `cargo check --workspace`：通过。
- `git diff --check`：通过。
- 前置提交 `e01ae61` 的隔离 package-test 证据：
  `http-session` 19/19、`track` 4/4 通过。
- 本分支再次运行 `SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f202 npm run
  test:http-session` 时，测试在预期失败的 wrong-ref 探针启动隔离 Router 阶段被本机
  Router 进程退出阻断，尚未进入业务用例；日志显示是 isolated supervisor readiness
  失败，不是数据库执行失败，也未操作 shared stable instance。

## 下一独立 owner

Account 在 `e01ae61` 上已消除 legacy projection 错误，18/19 测试通过。剩余一例是
assembly native 调用 `std.http.client.request` 未命中 test double，继而真实联网返回
503。该问题属于 assembly native test-double dispatch 路由，不属于数据库执行投影；
应作为后续独立任务处理。
