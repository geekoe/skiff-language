# P5-F311 Native platform catch consumer

状态：Completed。结果见
`P5-F311-native-platform-catch-consumer-result.md`。

## 直接父节点

- capability前置：`P5-F309-capability-platform-catch-consumer-result.md`
- complete mapping：`P5-F305-platform-catch-consumer-audit-result.md`

## DAG位置与范围

- 节点：platform catch DAG R3；与service-db、representation shared model并行。
- 唯一production范围：`runtime/native/**`。
- 允许co-located tests；禁止修改model/eval/capability/service-db/request/host。

## 完成标准

- `WirePayload::catch_projection`返回exact `CatchIdentity`；
- fixed variants映射：
  - BytesDecode、DbDecode、File、Http、Cancel、Timeout；
- dynamic decode code只通过
  `decode_target_error_code(...).and_then(PlatformBuiltinErrorIdentity::from_symbol)`；
- `std.resource.ResourceError`明确返回`None`，不能因payload code/public source存在而伪造platform
  identity；
- ordinary diagnostics保持`None`，Opaque继续转发inner projection；
- tests把Time/File/Cancel/Timeout等断言改为exact enum；Resource断言为`None`；
- test-only opaque delegation不向platform registry添加`test.*`字符串；
- 删除旧`TypeIdentity`与string identity构造，不改变payload bytes、display或cancel/timeout选择。

## 验证owner

```bash
cargo test -p skiff-runtime-native --lib -- --list
cargo test -p skiff-runtime-native --lib --no-fail-fast
git diff --check
```

selector非零。不运行eval/downstream/workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f311-native-catch`
- branch：`codex/p5-f311-native-catch`
- 一次性Agent，5分钟内修改；提交并返回mapping/Resource/forwarding矩阵与验证；
- 不push、不承接eval。
