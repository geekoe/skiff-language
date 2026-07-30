# P5-I02E：Skiff Consumer Combined Diagnostic

DAG节点I02E，依赖I51 PASS。exact candidate为commit
`e3b93c4ef6907d59e3a58e7ab17448ccec34c4d0`、tree
`7448c83a8e322f7631269a9111518ecb0ba88f30`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

全新只读gate owner在clean/no-writer状态只运行一次：

```bash
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-i02-cargo.XXXXXX)"
CARGO_TARGET_DIR="$P5_CARGO_TARGET" \
  node scripts/run-package-service-ecosystem-smoke.mjs --probe skiff-cutover --replicas 1
git diff --check
```

PASS要求与I02D相同的完整transaction/spawn-submit/withdrawal/tamper/reject/rollback ledger。FAIL必须把
`isolatedRuntimeLogEvidence`连同bounded terminal cause写到worktree外证据文件，据内部Router/Runtime日志给出
最后协议事件、唯一owner与遮挡范围；不得重试或修复。临时target/workspace/PID/ports全部清理。禁止R05、
stable、额外instance/full gate、编辑/提交；不作R02 verdict。
