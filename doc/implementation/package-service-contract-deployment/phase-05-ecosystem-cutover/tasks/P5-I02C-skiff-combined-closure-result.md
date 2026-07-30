# P5-I02C：Skiff Consumer Combined Closure Result

结论：FAIL。冻结docs HEAD `21e479fa4039bbc5bb06574adb61837b810ccac7`、production commit
`ad847f7254521d1dd4679a4f8af72b2c88753310`。唯一一次完整smoke已完成且清理。

fixture prepare/admit/commit、assembly readiness与首次typed unary进入Runtime后返回HTTP 500
`InvalidArtifact`：

```text
recoverable local concrete owner lookup found duplicate package id
test.skiff/package-service-i02-spawn-submit
```

直接失败点为`runtime/eval/src/recoverable_behavior.rs`的canonical assembly recoverable-owner projection。
typed submitted receipt、最终业务结果、withdrawal、tamper reject/abort及rollback ledger被遮挡。
bounded ledger与完整日志保存在worktree外`/Users/geek/workspace/skiff-phase-05-evidence/`。
