# P5-F20B：`skiff test` Explicit Binary Selection

全新开发Agent，从D26 docs checkpoint建立`/Users/geek/workspace/skiff-p5-f20b-test-bin`、
`codex/p5-f20b-test-bin`。一个clean commit；不merge/push/stable，不运行真实Cargo/test runner/I16/H18/full/Host。

exclusive write set：`scripts/skiff.mjs`中test caller与直接argv tests；不得改Cargo manifest/default-run、source-suite、
runtime-live/encrypted-storage/T06面、compiler/runner/Router/Runtime或lock。

完成态：公开`skiff test` production caller在`cargo run --locked --manifest-path <test-runner/Cargo.toml>`后显式传
`--bin skiff-test-runner`，再传`--`与原有runner argv；relative/absolute root、platform-source-root、profile/artifact-root
顺序与错误语义不变。argv test必须在两个binary且无default-run的真实manifest事实下锁定exact一次bin selector，并负断言
未新增default-run/环境fallback。不要泛化修改其他caller。

只运行直接Node argv tests、相关node check与`git diff --check`；不运行真实`skiff test`。报告matched>0、commit/tree/lock、
exact argv、T06排除、extra-review与clean。需越写集时停止。
