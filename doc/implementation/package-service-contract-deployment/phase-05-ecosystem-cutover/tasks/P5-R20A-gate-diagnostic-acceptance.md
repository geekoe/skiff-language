# P5-R20A：Gate Diagnostic Acceptance

使用未参与F19A/F20A、D25/D26、I16或其它验收的全新独立只读Agent。输入为F19A/F20A results、P26S与同一final
candidate/lock上的v5 I16 PASS bundle；第一行只给`R20A PASS/FAIL`。

必验：combined strict/full root-dep-info边界仍正确；v5在Host code/signal非零与零result line时保存可定位phase/subject、
byte counts及bounded diagnostic，同时路径/secret/body不泄漏；unknown也有首错。validator重算artifact与diagnostic，旧v4
fail closed；primary-first、attempt count、exact PASS+11/11+1/1语义不变；无第二parser/evaluator或production改动。

```bash
node --test --test-name-pattern 'full artifact evidence|Host attempt|bounded diagnostic|Host primary failure|legacy v4|v5 validator' \
  scripts/tests/platform-source-shared-target-probe.test.mjs
```

报告matched>0/pass/fail/skipped；不跑全部开发矩阵、I16/H18/Host/full/stable，不修改提交。运行extra-review并回报
identity/ledger/redaction与残余风险。
