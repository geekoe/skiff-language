# P5-I02F：Skiff Consumer Combined Final

依赖I53/I53A combined PASS。exact production candidate为commit
`ee21b85ddd70c63585af6961ce4ea1ef8d4ec37e`、tree
`e67a9f23f43b23a26b1915230fa592935f55b7d2`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

全新只读gate owner在clean/no-writer状态只运行一次：

```bash
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-i02-cargo.XXXXXX)"
CARGO_TARGET_DIR="$P5_CARGO_TARGET" \
  node scripts/run-package-service-ecosystem-smoke.mjs --probe skiff-cutover --replicas 1
git diff --check
```

PASS要求完整transaction/spawn-submit/withdrawal/tamper/reject/rollback ledger。FAIL必须保存bounded
`isolatedRuntimeLogEvidence`、最后内部协议事件、唯一owner与遮挡范围。不得重试/修复；target/workspace/PID/ports
全部清理。禁止R05/stable/额外instance/full gate/编辑提交；不作R02 verdict。
