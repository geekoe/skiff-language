# P5-I02C：Skiff Consumer Combined Closure

DAG节点I02C，依赖权威设计`doc/architecture/package-service-contract-deployment.md` §1–§15、
R05C PASS，以及D48批量修复后的I48/I48A combined PASS。exact production candidate为commit
`ad847f7254521d1dd4679a4f8af72b2c88753310`、tree
`f0a33cc750025916df7b303e2f07b9db3f2e9c6d`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

这是D48收敛后第三次且唯一一次完整combined。全新只读gate owner在完全clean、无writer状态只运行一次：

```bash
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-i02-cargo.XXXXXX)"
CARGO_TARGET_DIR="$P5_CARGO_TARGET" \
  node scripts/run-package-service-ecosystem-smoke.mjs --probe skiff-cutover --replicas 1
git diff --check
```

PASS必须记录完整transaction/spawn-submit/withdrawal/tamper/reject/rollback ledger，包括tooling fixture经
prepare/admit/commit进入Router/Runtime、四对象与activation/generation/assembly/exact replica/capability、
typed submitted receipt、最终业务结果、两次artifact-root withdrawal后旧unary、transitive PackageArtifact
tamper typed reject/abort，以及旧committed tuple/result/replica/capability/pending保持不变。

失败时返回bounded terminal cause、唯一blocking owner与已遮挡范围；不得重试或修复。临时Cargo target、
isolated workspace、PID及ports必须清理，并把完整ledger写到worktree外
`/Users/geek/workspace/skiff-phase-05-evidence/`。禁止编辑、提交、R05、stable、额外instance/full gate。
第一行`I02C PASS`或`I02C FAIL`；不作R02 verdict。
