# P5-F290 Open error effect consumer result

状态：`PASS`；实现完成，聚焦 test binary 暂由尚未合流的 artifact/language consumers 遮挡。

## Exact candidate

- implementation commit：
  `cbc0f6009566b4c2f9b554c0223af064dc82d874`
- 直接父任务：
  `P5-F290-open-error-effect-consumer.md`

## 结果

- `detached_contract_callee` 不再读取 `BoundaryErrorContract` 或 `contract.errors`。
- 满足既有 detached guarantees 的 service call 同时产生：
  - return origin `[Fresh]`；
  - direct return origin `[Fresh]`；
  - throw origin `[Fresh]`。
- `may_suspend` 只沿用 contract suspension fact，不改变 provenance。
- `detached_error`、unary、no-callback、detached parameter/return、no mutation/escape/same-heap
  等 gate 全部保持；任一必要 guarantee 缺失仍 fail closed。
- caller alias、caller mutation、escape、throw alias 和 same-heap identity 没有被开放错误通道伪造。
- production owner 反向搜索不再存在 closed error-set spelling。

## 证据与遮挡

```text
rustfmt --check                         PASS
git diff HEAD^ --check                  PASS
callable_effects --list / focused test  BLOCKED before test binary
```

遮挡来自当时尚未迁移的：

- F288-owned artifact identity `BoundaryErrorContract` / `operation.errors` /
  `signature.throw_types` consumers；
- F286-owned source/core declaration、union 与 contract-call consumers。

因此测试断言已经随实现提交，但不能把“未执行”记为通过。F288 与 F286 合流后的 combined compiler
probe 必须实际列出并执行 callable-effects tests；该 probe 才是本结果的动态证据 owner。
