# P5-I02A：Skiff Consumer Combined Final

权威设计为
`doc/architecture/package-service-contract-deployment.md` §1–§15。

DAG节点I02A，依赖F45A–F46A、I35关闭、I36 PASS及R05C PASS。它是修复旧I02证据入口后的新candidate唯一
one-replica combined owner，不作R02 verdict。

exact production candidate为commit
`95296242921cf26dfe961a735f652a84caf249b4`、tree
`2768f0822ed68ad511723442a45604e18a32c115`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。worktree必须完全clean且无在途写入。

全新只读Agent只运行一次：

```bash
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-i02-cargo.XXXXXX)"
CARGO_TARGET_DIR="$P5_CARGO_TARGET" \
  node scripts/run-package-service-ecosystem-smoke.mjs --probe skiff-cutover --replicas 1
git diff --check
```

必须报告：

- canonical std/fixture authoring与四对象identity；
- activationId/generation/assembly/exact replica/capability；
- canonical spawn submit typed response进入业务result；
- 两次artifact-root withdrawal下旧unary继续成功；
- transitive PackageArtifact tamper导致typed load reject/abort；
- committed tuple、旧result、replica/capability、pending不变；
- exact commit/tree/lock ledger、deadline及Cargo target/isolated cleanup。

R05C lifecycle证据沿用，不重复。禁止试跑、重试、编辑、提交、旧smoke、stable、额外instance或full gate。第一行
`I02A PASS`或`I02A FAIL`；FAIL给最小反例/唯一owner并停止。PASS只解除R02。
