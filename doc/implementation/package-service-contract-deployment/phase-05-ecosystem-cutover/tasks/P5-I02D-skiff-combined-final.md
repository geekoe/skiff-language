# P5-I02D：Skiff Consumer Combined Final

DAG节点I02D，依赖权威设计`doc/architecture/package-service-contract-deployment.md` §1–§15、
R05C的generation/wire证据、D49/F49/I49闭合。exact production candidate为commit
`42f322364f46f0be9350f4535ff492a562e73ae1`、tree
`9692c132cd07b06a1935772d63deea1ec86467c3`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

这是D49修复后的唯一完整combined。全新只读gate owner在完全clean、无writer状态只运行一次：

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
第一行`I02D PASS`或`I02D FAIL`；不作R02 verdict。
