# P5-R19A：Gate Evidence Acceptance

使用未参与F19A、D25、I16或其它验收的全新独立只读Agent。输入为F19A commit/result、同一final candidate/lock与v4
I16 PASS bundle；前后tracked clean且唯一untracked为该ledger。第一行只给`R19A PASS/FAIL`。

必验：combined严格hash/mtime语义未放宽；full只接受exact root-specific顶层dep-info materialization且记录diff，稳定
payload/mtime、缺Fresh、Fresh与Dirty/Compiling冲突均拒绝；失败ledger不丢before/outcome/after/first diff。Host command
发起即计1，nonzero/signal/parse/cleanup双错误仍可审计；success必须实际解析exact PASS line、11/11与1/1，不能硬编码。
v4 validator重算证据，旧v3 fail closed；无第二comparator/evaluator或production改动。

唯一抽查：

```bash
node --test --test-name-pattern 'full artifact evidence|full Host attempt|Host primary failure|unreachable expected assertion|legacy v3|v4 validator' \
  scripts/tests/platform-source-shared-target-probe.test.mjs
```

必须报告matched>0与pass/fail/skipped，不能用全skipped exit0；不重跑全部27项、I16/combined/H18/Host/full/stable，
不修改/提交。运行extra-review并回报identity/ledger/changed paths、blocker与残余风险。
