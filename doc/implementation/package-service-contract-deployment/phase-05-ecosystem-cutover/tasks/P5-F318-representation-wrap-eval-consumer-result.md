# P5-F318 Representation wrap eval consumer result

状态：PASS。

实现提交：`82cd078e0851a0d7394660ba33af4abe9204b21a`。

## 结果

- `LinkedExprIr::RepresentationWrap`只求值child一次；按fully-instantiated linked plan验证target与payload。
- 返回值保留原raw `RuntimeValue`，只替换最外层exact representation carrier identity。
- plain、generic、nested、external Package owner及ordered arguments全部保持精确；missing/wrong plan以及
  payload value/identity conflict失败关闭。
- named-union promotion只在目标上下文中的concrete branch identity与actual nominal identity精确相等时发生；
  两个enclosing union、不同泛型参数、同shape不同nominal、literal与synthetic branch不会混淆。
- ordinary representation materialization行为不变。
- direct throw/catch使用actual identity；wrap不创建site/frame，source、stack、correlation及同一Exception
  rethrow语义保持F299定义。

## 验证

- 独立representation consumer test list：6，非零。
- 独立representation consumer tests：6/6 PASS。
- `cargo check -p skiff-runtime-eval --lib`：PASS。
- targeted `rustfmt --check`与`git diff --check`：PASS。
- diff只含四个授权production文件及一个新integration test；F317的三个fixture没有被修改。
- wrap arm中的child求值调用恰好一次；没有legacy/compat/fallback、display/static throw或
  `payload_type`推断路径。

规定的eval `--lib` list/full命令已经执行，但被五个既存fixture API漂移遮挡：

- `test_effect_registry.rs`一处缺少`runtime_to_wire_required_plan`；
- `ordinary/tests.rs`一处旧`discriminator`与两处旧`throw_types`；
- `spawn_ops.rs`一处旧`discriminator`。

这些只作为F320合流状态combined probe的一批机械fixture closure处理，不改变本节点production结论。

