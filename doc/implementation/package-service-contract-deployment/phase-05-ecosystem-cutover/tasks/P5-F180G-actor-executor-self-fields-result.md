# P5-F180G：Actor 方法执行器与 Self 字段结果

状态：Completed

## 直接父任务

- `P5-F180F-runtime-actor-instance-store-result.md`

## 结果

已建立 Router 准入后的 Runtime 专用 Actor 方法交接与同步执行路径：

- 外部 `actor.method.invoke` 只能解码，不能直接取得执行权限；
- Router 准入后的内部交接必须同时携带目标 Runtime、owner lease、epoch、Actor ABI、implementation、
  declaration owner 和可选首次 activation bootstrap；
- Host 在调用执行器前再次核对完整 owner fence，校验失败时不会调用执行器；
- Actor 方法只按 linked declaration 的公开 method identity 解析私有实现入口，不回退普通 service
  request，也不接受调用者提供 `ExecutableAddr`；
- 参数和返回值分别按 linked public method signature 与现有 boundary codec 解码、编码。

每个 Actor 实例拥有独立 scheduler。同一实例一次只执行一个同步片段，不同实例的 scheduler
彼此独立。每次取得执行权都会生成 crate-private execution token；token 与精确实例、epoch 和
执行 lease 绑定，执行结束后立即失效。

Actor `self` 字段读写已改为专用 artifact/linked IR：

- 只有 Actor 实现方法可以产生并链接该 IR；
- linker 重新校验声明、字段名和字段类型，普通函数、常量及伪造 artifact 全部失败关闭；
- eval 只有在当前 Actor execution token 下才能读取或写入字段；
- 写入先按 linked field type plan 做 boundary round trip 校验；
- 方法在事务快照上执行，只有返回值也通过类型编码后才原子提交 live fields 与 heap；
- 错误方法、错误参数、字段类型错误、返回类型错误或执行错误均不会改变原值。

执行前和提交前都会重新校验实例 identity、epoch、ABI、implementation 与声明 owner。声明为
`maySuspend` 的方法以及实际出现 `Flow::Parked` 的同步执行会返回明确的
`CoroutineNotImplemented`，并释放 scheduler 和 execution token；F180G 不持有同步 mutex
跨异步执行，continuation 与恢复 fence 留给 F180H。

编译器侧同时修复了两个被旧测试遗漏的问题：

- 隐式 `self` 的 Actor 方法不再错误丢弃第一个业务参数；
- `self.<field> = value` 现在校验右值是否可赋给声明字段类型。

本任务没有实现 Router upgrade、崩溃恢复、TTL 或协程 continuation。

## 验证

- Actor executor 真实 linked 全链：4/4 PASS
  - 连续两次真实方法调用返回 `8`、`13`，证明 live 字段跨调用提交；
  - 错误 method、参数数量、参数类型、返回类型、ABI、implementation、epoch 全部失败关闭；
  - `maySuspend` 后可在 100ms 内重新取得同实例 scheduler；
  - 普通执行上下文无 token 时 Actor 字段访问失败。
- Actor instance/store/scheduler：16/16 PASS
  - 同实例同步执行 lease 串行；
  - 不同实例通过 barrier 证明可并行；
  - 失败快照回滚、字段类型保护和 token 失效均通过。
- Runtime Host handoff：9/9 PASS。
- Compiler source Actor 聚焦测试：6/6 PASS。
- Compiler lowering Actor 聚焦测试：1/1 PASS。
- Runtime linker Actor self IR：3/3 PASS。
- `cargo check --workspace`：PASS。
- 本任务修改的 Rust 文件 `rustfmt --check`：PASS。
- `git diff --check`：PASS。

