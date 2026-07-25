# P5-F301 Applied nominal package/public compiler consumer结果

状态：Implemented checkpoint。

任务提交：`3eb0cbf50bb61589e7a8942798374f635ea626a3`。

集成提交：`b59eac463a23f4decdb64a9fee0c4ed3730c8cd7`。

## 直接任务与权威链

- `P5-F301-applied-nominal-package-public-consumer.md`
- 任务继续引用F296、F295与F293父链。

## 结果

- compiled projection input、canonical dependency binding、callable/public instance与
  implementation link保留`AppliedNominal` wrapper、ordered arguments和exact owner；
- `PackageSymbol` base与nested arguments递归绑定正确Local ABI expectation；
- package ABI declaration handoff保留ordered type parameters、五种declaration kind与三种
  named-union branch，不再以旧anonymous `variants`/discriminator DTO恢复；
- package link/Local ABI允许fully-instantiated applied nominal；
- local/dependency PackageSchema、service boundary与public error schema候选遇generic时在写入任何
  partial index/record前显式fail closed；
- `ResolvedPackageSchema`拒绝forged non-empty canonical `type_params`；
- current-generation applied `PackageSchema`继续由F295 strict admission拒绝。

## 验证

- compiled list/full：PASS，5/5；
- projection-input list/full：PASS，9/9；
- projection list/full：PASS，50/50；
- `git diff --check`：PASS；
- production反搜旧`Union { variants }`、`.variants`、discriminator flattening与重复
  `PackageAbiType` DTO：零命中。

开发分支上的compiler integration在枚举前被当时尚未合入的F300 runtime linker旧consumer遮挡。
F300现已先行合入integration；下一节点必须在本集成提交上运行一次compiler combined probe，再进入
`A2-language`独立验收。

