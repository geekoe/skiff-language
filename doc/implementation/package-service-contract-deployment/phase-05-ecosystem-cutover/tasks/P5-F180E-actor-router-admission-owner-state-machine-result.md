# P5-F180E：Actor Router 接纳与 Owner 状态机结果

状态：Completed

## 直接父任务

- `P5-F180D-actor-method-wire-error-contract-result.md`

## 结果

Router 已建立 logical Actor 的原子 owner 与方法接纳状态机：

- registry entry 明确保存 `inactive / activating / live / upgrading`；
- owner 获取会原子检查 logical key、epoch、implementation 和现有 lease 到期时间，未过期 owner
  不能被另一个 Runtime 覆盖；
- owner 激活、续租、释放都要求完整的 epoch、implementation、Runtime 和 lease fence；
- 相同 implementation 只复用当前 `live` 且 lease 有效的 owner；
- 第一个不同 implementation 调用原子关闭新接纳并进入 `upgrading`，目标 implementation 收到
  `ActorUpgradingError`，旧实现或第三种实现收到 `ActorVersionRejectedError`；
- stale epoch 返回 `ActorIncarnationReplacedError`；Actor ABI 和 method identity 通过独立、权威的
  method catalog 边界精确校验，未知值 fail closed；
- Actor dispatcher 只接收 F180D 的 `actor.method.invoke`，并把完整 owner fence 交给专用 owner
  transport；普通 service request 或其他 Actor frame 不能进入该路径；
- invocation ledger 独立记录 `admitted / dispatched / completed / cancelled / failed`，所有转换都校验
  invocation、logical Actor、epoch、implementation、owner Runtime 和 lease，重复或越序转换拒绝；
- store 暴露按 owner fence 批量失败调用和原子释放 owner 的接口，供后续 Runtime 断连恢复任务使用。

调用 ledger 已从 registry entry 存储中拆为独立职责，避免继续扩大原有 in-memory registry 文件的
职责混杂。本任务没有实现 Runtime ActorInstanceStore、方法执行器、协程恢复、upgrade drain、崩溃
恢复或 TTL sweeper。

## 验证

- `pnpm --dir router type-check`：PASS
- `pnpm --dir router exec vitest run tests/actor-router-admission.test.ts tests/actor-manager.test.ts tests/actorMethodProtocol.test.ts tests/actor-spawn-runtime-control.test.ts`：29/29 PASS
- Router 全量测试：447 PASS、88 SKIP、4 FAIL；失败来自既有 compiler authoring fixture、
  artifact identity CLI 构建路径和 spawn queue 时间夹具，与本任务 Actor Router 改动无关。
- `git diff --check`：PASS

聚焦测试覆盖两个 Runtime 并发抢 owner、未过期 lease 防覆盖、错误 fence 续租/释放、同实现复用、
不同实现升级关闭接纳、stale epoch、错误 ABI/implementation/method、ledger 合法与非法转换，以及
普通 frame 不能进入 Actor dispatcher。
