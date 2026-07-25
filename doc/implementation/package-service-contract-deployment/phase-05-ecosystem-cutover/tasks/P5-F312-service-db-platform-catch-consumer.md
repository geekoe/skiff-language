# P5-F312 Service DB platform catch consumer

状态：Completed with unrelated suite blockers。结果见
`P5-F312-service-db-platform-catch-consumer-result.md`。

## 直接父节点

- capability前置：`P5-F309-capability-platform-catch-consumer-result.md`
- complete mapping：`P5-F305-platform-catch-consumer-audit-result.md`

## DAG位置与范围

- 节点：platform catch DAG R4；与native、representation shared model并行。
- 唯一production范围：`runtime/service-db/**`。
- 允许co-located tests；禁止修改model/eval/capability/native/request/host。

## 完成标准

- `WirePayload::catch_projection`返回exact `CatchIdentity`；
- Mongo conflict使用`PlatformBuiltinErrorIdentity::DbConflict.catch_identity()`；
- Opaque继续转发inner projection；
- 其它service-db errors保持`None`；
- 特别是payload code为`std.db.DecodeError`的DbDecode仍保持`None`，不能按字符串code猜identity；
- 删除旧`TypeIdentity` import/string构造，不改变payload、display或Mongo conflict分类。

## 验证owner

```bash
cargo test -p skiff-runtime-service-db --lib -- --list
cargo test -p skiff-runtime-service-db --lib --no-fail-fast
git diff --check
```

selector非零。不运行eval/host/downstream/workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f312-service-db-catch`
- branch：`codex/p5-f312-service-db-catch`
- 一次性Agent，5分钟内修改；提交并返回conflict/None/forwarding矩阵与验证；
- 不push、不承接host。
