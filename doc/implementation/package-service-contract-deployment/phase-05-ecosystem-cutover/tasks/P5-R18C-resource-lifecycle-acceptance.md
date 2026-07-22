# P5-R18C：Resource Lifecycle Acceptance

使用未参与F17、F18C/E/F、D19/D20、I16或其它验收的全新独立只读Agent。权威设计：架构§3、§6.1、§6.2、
§11、§14。输入为同一final candidate/lock、F17与F18C/E/F ledgers、I16 PASS bundle；I16 path/registry/task-root/
PID/port/temp cleanup均ABSENT。第一行只给`R18C PASS/FAIL`。

必验：

- F18C只计算一次absolute root/target并覆盖hostile env；config/bootstrap/supervisor/runner一致；readiness要求exact active
  tuple、healthy connected replica、同ID connected capability，missing/false拒绝。
- F18E/F17在首个await前由唯一lifecycle接管；false-stop/仍alive reject、保留PID、禁止restart，handles all-settled；
  unsupervised failure进入同一completion且primary-first。PID nonce+inode no-clobber，foreign/replacement保留。
- F18F以nonce、path inode、claim、Git registry/admin identity与task-root marker证明ownership；partial add只清自己，
  禁止force/recursive foreign cleanup；ledger为wx+flush+close+hard-link no-clobber。

唯一抽查：

```bash
node --unhandled-rejections=strict --test \
  --test-name-pattern='readiness requires one exact connected replica|one absolute checkout and Cargo target flow|false process-group stop rejects|pre-existing and concurrent PID claims|task-root replacement preserves|ledger destination race preserves' \
  scripts/tests/isolated-test-runtime.test.mjs \
  scripts/tests/skiff-instance-supervisor-lifecycle.test.mjs \
  scripts/tests/skiff-instance-pid-metadata.test.mjs \
  scripts/tests/platform-source-probe-ownership.test.mjs
```

必须报告实际matched tests>0，不能用全skipped exit0冒充。复用F17/F18C/E/F完整矩阵与I16 cleanup，不重跑Host/I16/
完整Router/Runtime。foreign被删/覆盖、cleanup盖primary或任何child/FD/PID/port/temp残留即FAIL；回报extra-review与失效范围。
