# P5-R20B：`skiff test` Binary Acceptance

使用未参与F20B/D26/I16或其它验收的全新独立只读Agent。输入为F20B result、P26S与同一final candidate/lock上的v5
I16 PASS bundle；第一行只给`R20B PASS/FAIL`。

必验真实manifest恰两个bin且无default-run；production test caller显式exact一次`--bin skiff-test-runner`，位于Cargo选项与
`--`之间；absolute/relative、live/non-live、profile/artifact/platform/base/strict顺序与错误语义不变。hostile env不能改
selector；runtime-live/encrypted/T06与manifest/lock零diff。

```bash
node --test --test-name-pattern 'canonical binary once' scripts/tests/skiff-test-cli.test.mjs
```

报告matched>0；不运行真实Cargo/`skiff test`/I16/H18/Host/full/stable，不修改提交。运行extra-review并回报identity/ledger、
exact argv与残余风险。
