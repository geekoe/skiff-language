# P5-F313 Linked-type-plan service error index fixture migration

状态：Ready。

## 直接父节点

- `P5-F310-linked-type-plan-platform-catch-consumer-result.md`
- `P5-F298-service-error-type-index-result.md`

## 范围与完成标准

唯一写入文件：`runtime/linked-type-plan/src/assembly_seam.rs`。

- 测试构造`AssemblyExecutionImage::try_new`时显式传入
  `Arc<ServiceErrorTypeIndex::default()>`；
- 补齐必要import；
- 不改production API、错误索引语义、测试断言或其它fixture；
- 反搜该crate不再有旧3参数调用。

## 验证owner

```bash
cargo test -p skiff-runtime-linked-type-plan --lib -- --list
cargo test -p skiff-runtime-linked-type-plan --lib --no-fail-fast
git diff --check
```

selector非零。不得修改其它crate或处理其它遮挡。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f313-linked-type-plan-fixture`
- branch：`codex/p5-f313-linked-type-plan-fixture`
- 一次性Agent，5分钟内修改，提交并返回测试；
- 不push、不操作stable、不承接其它节点。

