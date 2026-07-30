# P5-F18J：Authoring Platform Guard Before Store IO

## 输入与 owner

这是 R18A 在 candidate `ecc53ec27c493e692f03112ba7d951397fadd831` 上发现的独立修复节点。权威语义仍为
`doc/architecture/package-service-contract-deployment.md` §3、§9、§10、§14；不得改变公共契约或平台根语义。
使用全新开发 Agent，从本任务合同 checkpoint 建立
`/Users/geek/workspace/skiff-p5-f18j-authoring-pre-store`、分支
`codex/p5-f18j-authoring-pre-store`。一个 clean commit；不 merge/push/stable，不运行 I16、Host 或 full suite。

exclusive write set：`compiler/driver/authoring.rs` 及其 child tests，和
`test-runner/src/canonical_package/tests/combined.rs`。不得改 compiler input/source/pipeline、artifact store 实现、
runner production、manifest 或 lock。

## 完成态

- package object 分支在第一次 manifest/source/store/dependency IO 前执行既有 platform-root guard；
  different-root 返回 typed `DifferentPlatformRoot`，且不得创建、canonicalize 或清理调用方传入的 artifact store 路径。
- 非 object 分支不被无故要求 platform sources，现有行为不变；same-root object authoring 仍只初始化一次 prelude registry，
  pipeline guard保留为 defense-in-depth。
- 修复 guard/store 初始化顺序，不新增 root comparator/resolver，不字符串化 typed error，不把 store creation 搬给调用方。
- combined regression 必须在调用后断言 hostile `authoring_store` 从未存在；不能用“创建后再清理”掩盖副作用。

## 验证与交付

运行最窄验证，不运行 merge-only combined、I16、Host 或 full：

```bash
cargo test --locked -p skiff-compiler --lib p5_f18b_authoring_mismatch_zero_source_reads -- --test-threads=1
cargo check --locked -p skiff-compiler --lib
cargo test --locked -p skiff-test-runner --lib --no-run
git diff --check
```

回报 parent/commit/tree/lock、changed paths、typed downcast、store 路径不存在的测试证据、same-root/非 object 顺序核验、
反搜第二 resolver 与 extra-review。若必须越过 exclusive write set 或改变公共契约，停止并报告设计决策。
