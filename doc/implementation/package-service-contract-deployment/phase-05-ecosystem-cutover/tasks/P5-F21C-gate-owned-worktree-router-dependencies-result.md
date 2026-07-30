# P5-F21C：Gate Owned Worktree Router Dependencies Result

`F21C PASS`

开发提交为`39328d5461b148ed310a474df6c45e5a0bf5965c`，parent
`714978aeb3062052f74e5f81420b4f8da44e53a3`，tree
`77455c412a1922b82a0340b70507934006da3848`。bit-identical合流提交为
`f14374ee5f26f0394eef56fc7881a896f2879cc2`，parent
`e9841990ec7e49e15036f7e9856e3735294ac3fd`，tree
`9f14b5136656dff707e244d10cb997e370e4cc5f`；`Cargo.lock` blob保持
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

新增的单一child owner只在full模式的A→B artifact evidence PASS之后、真实Host attempt之前，从owned B cwd执行
`pnpm --dir router install --frozen-lockfile --offline`，随后直接执行B-local
`router/node_modules/.bin/tsx --version`。combined模式调用数为0；install、tsx spawn/nonzero/signal均在Host前
fail closed，并把有界code/signal、bytes与SHA-256 outcome交回原Gate ledger。实现没有借用integration、A、home或
foreign `node_modules`，也没有修改Router/Runtime、manifest、lock、artifact comparator或公共语义。

command-double聚焦验证16 pass、0 fail；两个Node syntax check与`git diff --check`均PASS。顺序、full-only、B cwd、
locked/offline argv、dependency/tsx失败阻断Host、primary failure优先于cleanup及owned cleanup分支均被覆盖。规则集中在
113行的dependency child owner，没有复制到evaluator/validator/helper；未运行真实install、Cargo、I16、Host/full或
stable。本结果只交付F21C实现checkpoint，不是G16、F04或阶段verdict。
