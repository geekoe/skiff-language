# P5-F323 Eval fixture schema and heap closure result

状态：PASS for 19+1 scope；完整eval仍有三个独立blocker。

实现提交：`ba6e3971`。

## 结果

- `link_package_fixture`为无public schema record的Package注入package id和identity精确匹配的空
  `PackageSchemaIndex`；发现非空record时显式拒绝，不能用空index掩盖真实schema。
- F320列出的19个`MissingHydratedSchemaIndex`失败全部清除。
- typed throw heap测试不再假定两个独立heap的数值handle必须不同；它通过payload、outer/item identity、
  双向修改隔离与rollback证明没有跨heap别名。
- 没有放宽production admission、修改WebSocket或representation。

## 验证

- eval list：154，非零。
- 完整eval：151/154 PASS。
- targeted `rustfmt --check`与`git diff --check`：PASS。

三个剩余失败：

1. `inline_effect_typed_throw_is_caught_by_exact_linked_nominal_type`已越过schema hydration，暴露fixture的空
   request trace id；
2. 两个source-inline fixture仍被generic WebSocket public-schema设计决定阻塞。

前者进入F325机械fixture closure；后两者等待用户决定后进入同一WebSocket compiler batch。

