# P5-F309 Capability-context platform catch consumer

状态：Completed。结果见
`P5-F309-capability-platform-catch-consumer-result.md`。

## 直接父节点

- `P5-F305-platform-catch-consumer-audit-result.md`
- `P5-F299-runtime-local-exception-carrier-implementation-result.md`

## DAG位置与范围

- 节点：platform catch DAG R1；完成后解除native与service-db。
- 唯一production范围：`runtime/capability-context/**`。
- 允许co-located tests；禁止修改model/eval/native/service-db/request/host/transport或其它crate。

## 完成标准

- 所有`WirePayload::catch_projection`返回`CatchIdentity`；
- 使用exact `PlatformBuiltinErrorIdentity::<Variant>.catch_identity()`映射：
  File、ServiceProviderUnavailable、ServiceProtocol、Cancel、Timeout；
- opaque/stream/execution/producer wrapper继续转发inner projection；
- decode、unsupported、resource limit与普通diagnostic保持`None`；
- cancelled budget映射Cancel，其它deadline/instruction budget映射Timeout；
- test-only opaque delegation不得向platform registry添加`test.*`字符串；使用显式test fixture identity或
  已有合法platform identity；
- 删除旧`TypeIdentity` import/string构造，无payload或cancel/timeout选择变化。

## 验证owner

```bash
cargo test -p skiff-runtime-capability-context --lib -- --list
cargo test -p skiff-runtime-capability-context --lib --no-fail-fast
git diff --check
```

selector非零。不运行eval/downstream/workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f309-capability-catch`
- branch：`codex/p5-f309-capability-catch`
- 一次性开发Agent，5分钟内修改；提交并返回mapping/forwarding矩阵与验证；
- 不push、不承接native/service-db。
