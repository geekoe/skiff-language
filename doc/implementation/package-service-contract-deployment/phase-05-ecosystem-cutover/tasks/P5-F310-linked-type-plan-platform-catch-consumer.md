# P5-F310 Linked-type-plan platform catch consumer

状态：Implemented，验证待F313关闭fixture遮挡。结果见
`P5-F310-linked-type-plan-platform-catch-consumer-result.md`。

## 直接父节点

- `P5-F305-platform-catch-consumer-audit-result.md`
- `P5-F299-runtime-local-exception-carrier-implementation-result.md`

## DAG位置与范围

- 节点：platform catch DAG R2；与R1/F307并行。
- 唯一production范围：`runtime/linked-type-plan/src/error.rs`。
- 允许该crate co-located tests；禁止修改model/eval/boundary/capability/native/request/host。

## 完成标准

- `WirePayload::catch_projection`返回`CatchIdentity`；
- protocol error使用
  `PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity()`；
- boundary wrapper继续精确转发inner projection；
- invalid artifact/普通diagnostic保持`None`；
- 删除旧`TypeIdentity` import/string构造，不改变payload bytes或error display。

## 验证owner

```bash
cargo test -p skiff-runtime-linked-type-plan --lib -- --list
cargo test -p skiff-runtime-linked-type-plan --lib --no-fail-fast
git diff --check
```

selector非零。不运行eval/downstream/workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f310-linked-type-plan-catch`
- branch：`codex/p5-f310-linked-type-plan-catch`
- 一次性开发Agent，5分钟内修改；提交并返回mapping/forwarding与验证；
- 不push、不承接其它节点。
