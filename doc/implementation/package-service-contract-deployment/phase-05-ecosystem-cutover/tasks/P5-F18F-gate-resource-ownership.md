# P5-F18F：Gate Resource Ownership

权威设计：`doc/architecture/package-service-contract-deployment.md` §3、§14；I16/G16与D20 result。从D20 docs
checkpoint建立`/Users/geek/workspace/skiff-p5-f18f-gate-ownership`、`codex/p5-f18f-gate-ownership`。全新Agent、一个
commit，不merge/push/stable，不运行真实combined/full/Host；五分钟内修改。

exclusive write set：`scripts/lib/platform-source-probe-{contract,preflight,support,ownership}.mjs`、
`platform-source-shared-target-probe.mjs`及对应gate/ownership tests。禁止触碰compiler/runner/Router/Runtime/fixture/
manifest/lock。

完成态：每次probe有nonce；任何可删除资源同时持有path inode、marker/claim与Git registry identity。partial add失败后只
清理可证明由本次新建的registry/path；preflight后foreign path、path/task-root replacement永不force/recursive删除。
task root marker+inode不匹配时保留并FAIL。ledger/temp用`wx`+flush/close+hard-link no-clobber安装，foreign destination
字节不变；所有失败清own temp。PASS ledger同时证明A/B path+registry、claim、task root、temp absence与foreign preserved。

```bash
node --test scripts/tests/platform-source-shared-target-probe.test.mjs scripts/tests/platform-source-probe-ownership.test.mjs
node --check scripts/lib/platform-source-probe-contract.mjs
node --check scripts/lib/platform-source-probe-preflight.mjs
node --check scripts/lib/platform-source-probe-support.mjs
node --check scripts/lib/platform-source-probe-ownership.mjs
node --check scripts/lib/platform-source-shared-target-probe.mjs
git diff --check
```

固定negatives：registry partial add、foreign A/B、post-add inode replacement、task-root replacement、ledger race、temp write
failure、remove failure与primary-first。回报commit/tree/lock、零foreign delete/overwrite、cleanup ledger与extra-review。
