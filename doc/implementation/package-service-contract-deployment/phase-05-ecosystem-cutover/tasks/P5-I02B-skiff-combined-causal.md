# P5-I02B：Skiff Consumer Combined Causal

DAG节点I02B，依赖F46B/I37 PASS及R05C有效。exact production candidate为commit
`00649e5b459913c957c28a437368bac8a9e48acf`、tree
`47b392ac42b8ec7563151ca4b5b35a107ef23a3f`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

全新只读Agent在完全clean、无writer状态只运行一次：

```bash
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-i02-cargo.XXXXXX)"
CARGO_TARGET_DIR="$P5_CARGO_TARGET" \
  node scripts/run-package-service-ecosystem-smoke.mjs --probe skiff-cutover --replicas 1
git diff --check
```

PASS要求完整transaction/spawn-submit/withdrawal/tamper/reject/rollback ledger。若fixture Cargo失败，必须返回新bounded
diagnostic保留的terminal cause与唯一owner；不得重试或修复。临时target/instance/PID/ports全部清理。禁止编辑、提交、
R05、stable、额外instance/full gate。第一行`I02B PASS`或`I02B FAIL`；不作R02 verdict。
