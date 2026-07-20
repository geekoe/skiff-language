# P4-F03：Callback Capability Cleanup / Rollback Repair

## 权威输入、风险与证据状态

- 执行输入：R01在`ef14a08`的blocking issue 2；expired entry仍强持有payload且不从active table清除，callback
  materialization在注册后destination分配失败时没有revoke/rollback。
- 风险/验收组：高风险owner/lifetime/transaction；由R01复验，不解锁T06。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：T02 checkpoint与R01 FAIL；可与F02/F04并行。
- 解锁：R01 retry。
- branch：`codex/p4-f03-capability-cleanup-rollback`。
- worktree：`/Users/geek/workspace/skiff-p4-f03-capability-cleanup`。
- 五分钟内真实edit；原T02 owner执行。不得把payload放进tombstone、共享全局表或recoverable encoding。

## 写入范围与完成态

- 独占`runtime/activation` capability/request lifetime state与`runtime/boundary` callback projection transaction/rollback；
  可补model/service-db既有拒绝测试，不修改eval/host/linker/router。
- active entry与expired tombstone分离：expire/drain必须exact-once释放payload并从active count移除；后续lookup仍稳定
  返回`CapabilityExpired`而非unavailable/rebuild。tombstone必须有明确activation/generation owner与有界清理策略。
- request end/cancel按generation drain；stream lease存在时延长到最后close；owner exit drain全部owner entries；重复
  terminal/revoke/drop不重复释放。
- materialization注册与destination carrier allocation形成事务：后续任何失败都revoke/rollback已注册entry，成功
  时只commit一次。hook/API必须足够让T06消费，不让T06再改共享ABI。
- `DropProbe`/计数测试覆盖request、stream、cancel、owner exit、重复终止、allocation failure；每条路径payload
  exact-once drop、active count归零、fallback/rebuild调用零次。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-activation callback_capability_cleanup
cargo test -p skiff-runtime-boundary callback_capability_rollback
cargo test -p skiff-runtime-service-db callback_capability
git diff --check
```

不得运行完整runtime gate。

## 回报

提交一个clean commit，回报active/tombstone状态机、terminal→drain矩阵、rollback guard与drop计数证据。
