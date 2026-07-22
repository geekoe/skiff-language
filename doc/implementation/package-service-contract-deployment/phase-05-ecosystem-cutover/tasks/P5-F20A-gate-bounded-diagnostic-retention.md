# P5-F20A：Gate Bounded Diagnostic Retention

全新开发Agent，从D26 docs checkpoint建立`/Users/geek/workspace/skiff-p5-f20a-gate-diagnostic`、
`codex/p5-f20a-gate-diagnostic`。一个clean commit；不merge/push/stable，不运行真实Cargo/I16/H18/full/Host。

exclusive write set：`scripts/lib/platform-source-probe-evidence.mjs`、`platform-source-probe-contract.mjs`、新增单一
`scripts/lib/platform-source-probe-diagnostic.mjs`、`scripts/tests/platform-source-shared-target-probe.test.mjs`。不得改source suite、isolated
runtime、compiler/runner production、fixture、CLI、manifest/lock。

完成态：Host outcome在code/signal非零、零result line时仍保存规范化`phase/subject`、stdout/stderr byte counts与
`firstDiagnostic {kind,stream,sanitizedExcerpt,originalLineSha256,truncated}`。excerpt固定上限；integration/A/B/task/temp/
home路径替换为稳定token，secret sentinel、HTTP body与全量输出不得入ledger。利用现有`[skiff-test]`/`[skiff-tests]`
阶段标记区分startup/std/Host prepare/Host runner；无法分类也保留bounded unknown diagnostic。primary-first与原始hash不变。

command-double表注入四阶段code1、超长行、路径与secret，断言可定位、bounded、redacted且validator重算；旧v4显式失效，
schema升v5。诊断parser为单一child owner，不在538行evaluator继续堆职责。

```bash
node --test --test-name-pattern 'Host attempt|bounded diagnostic|legacy v4|validator' scripts/tests/platform-source-shared-target-probe.test.mjs
node --check scripts/lib/platform-source-probe-evidence.mjs
node --check scripts/lib/platform-source-probe-diagnostic.mjs
node --check scripts/lib/platform-source-probe-contract.mjs
git diff --check
```

报告matched>0、commit/tree/lock、redaction/phase矩阵、schema、extra-review与clean；需越界或公共语义时停止。
