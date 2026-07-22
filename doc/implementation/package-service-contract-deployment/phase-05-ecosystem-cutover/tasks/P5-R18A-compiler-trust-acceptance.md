# P5-R18A：Compiler Trust Acceptance

使用未参与F16A/B/C、F18A/B/H、D20、I16或其它验收的全新独立只读Agent。权威设计：架构§3、§9、§10、§14。
输入为同一final repair candidate/lock与I16 PASS bundle；merge-only必须`1 passed / 0 ignored`，A-built/Fresh-B、4次
identity/8个golden、hash/mtime、strings/dep-info no-match、registry与cleanup证据齐全。第一行只给`R18A PASS/FAIL`。

必验：

- `CompilerPlatformSources`是唯一canonicalize/contain/read owner；Prelude loader只消费immutable logical-path/text
  snapshot，root-outside symlink为typed `InvalidLayout`，same-root/排序/filter/golden不变。
- authoring/runner在首个manifest/source/store/dependency IO前guard；different-root typed zero-read，pipeline仅defense。
- F18H test common只构造一次test-only context并贯穿graph，无global/OnceLock/第二resolver/production ambient path；
  三处旧签名归零、18 targets compile evidence有效。

唯一抽查：

```bash
cargo test --locked -p skiff-compiler-source --lib p5_f18a_prelude_loader_snapshot -- --test-threads=1
```

不重跑F18A/B/H或I16矩阵、不跑Host/full suite。检查extra-review的第二reader/resolver与边界职责；候选、compiler
input/source/authoring/test-common、platform sources、manifest/lock或I16变化即失效。
