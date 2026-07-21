# P5-F04B：Source Suite Runner Disambiguation

## 输入、owner与完成态

- 输入：R11 PASS后exact integration `efb2bbbe1f2b81fc795fd6542849bf5a9e6f2627` / tree
  `70d3c8d31c2a748ff642c99f2f3c29947bf181c2`；真实isolated runtime已ready并进入std，Cargo因test-runner crate
  有两个binary且source suite未指定`--bin`而退出101。
- 独立worktree/branch，一个clean commit，不merge/push。
- owner只限`scripts/lib/skiff-source-test-suite.mjs`与直接Node test。不得改manifest/lock、Rust runner/fixture、
  Router/Runtime/compiler/shared wire或stable。

`skiffSourceTestRunnerCargoArgs`必须显式生成`cargo run --manifest-path <test-runner/Cargo.toml> --bin
skiff-test-runner -- ...`，保留现有root、artifact root、strict policy与generation传递；不按binary发现顺序猜测，也不删除
package-service fixture binary。直接test断言完整argv与实际crate多binary条件下仍选择canonical runner。

```bash
node --test scripts/tests/skiff-source-test-suite.test.mjs
node scripts/run-skiff-tests.mjs
git diff --check
```

关键门禁仍是后一个命令真实越过std、prepare、activation、HTTP Host ingress并执行checked-in consumer exact断言；
若暴露下一production blocker，保持本任务clean并给首个精确证据，不顺手扩大写域。
