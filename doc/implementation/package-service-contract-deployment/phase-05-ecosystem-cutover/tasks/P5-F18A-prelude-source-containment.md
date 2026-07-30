# P5-F18A：Prelude Source Containment

## 输入与owner

权威设计：`doc/architecture/package-service-contract-deployment.md` §3、§9、§10、§14；D18/F16A与D20 result。
从root宣布的D20 docs checkpoint建立`/Users/geek/workspace/skiff-p5-f18a-prelude-containment`、分支
`codex/p5-f18a-prelude-containment`；production基线为`e786671...`。使用全新开发Agent，一个clean commit，
不merge/push/stable，不运行I16/Host。五分钟内修改，否则`TASK_NOT_EXECUTABLE`。

exclusive owner为`compiler/input/src/platform_sources{,/tests}/**`与
`compiler/source/src/prelude_registry/{mod,initialization,loading,tests}/**`，可新增compiler pipeline聚焦test。
不改authoring/canonical_package、CLI/JS、Router/Runtime、std/prelude内容、manifest/lock。

## 完成态

- `CompilerPlatformSources`唯一枚举、canonicalize、contain并读取PreludeRegistry所需official `.skiff`；source loader只
  消费immutable `(logical path, text)` snapshot并负责parse/module/semantic，不再`read_dir/read_to_string`。
- root-outside symlink返回typed `CompilerPlatformSourcesError::InvalidLayout`并透明映射为
  `PreludeRegistryInitializationError::PlatformSources`；same-root symlink、排序与test-source过滤不变。
- registry/identity从同一snapshot派生；different-root guard仍先于snapshot IO；prelude/std golden bit-identical。
- 删除旧重复collector/read owner，禁止新增第二containment helper或反向crate依赖。

## 验证与交付

固定tests覆盖prelude+std root-outside真实symlink、same-root正例、loader不重读、真实compiler不发布escape artifact及golden。

```bash
cargo test --locked -p skiff-compiler-input --lib p5_f18a_platform_snapshot_containment -- --test-threads=1
cargo test --locked -p skiff-compiler-source --lib p5_f18a_prelude_loader_snapshot -- --test-threads=1
cargo test --locked -p skiff-compiler --lib p5_f18a_real_compiler_symlink_escape -- --test-threads=1
cargo check --locked -p skiff-compiler --lib
git diff --check
```

只对changed Rust运行rustfmt；global fmt baseline另列。回报parent/commit/tree/lock、changed paths、typed error、golden、
反向搜索、extra-review与clean状态。若需改公开错误、manifest、package_sources或F18B写集，停止。
