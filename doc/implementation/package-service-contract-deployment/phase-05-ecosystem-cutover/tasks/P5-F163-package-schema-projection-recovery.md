# P5-F163：Package Schema Projection Recovery

状态：Ready

## 直接父任务

- `P5-F162-compiler-package-schema-input-result.md`

## 继承事实

- F161 blocker审计见`P5-F161-package-schema-compiler-projection-result.md`。
- F160已提供content-addressed store，F162已提供exact resolved dependency schema input。
- 用户已决定第一版boundary named type必须在owner Package的`api.yml`显式公开。

## 范围

修改compiler PackageArtifact projection、compiler/contract以及直接相关projection fixtures。不得修改
compiler dependency consumer materialization、deployment、runtime或service consumer。

## 必须实现

- 当前Package从typed public API graph生成`PackageSchemaTypeRecord`、`PackageSchemaIndex`及artifact refs；
  compiler driver/publication把records/index写入canonical store。
- schema graph按公开path建图，先拒绝未公开named child与SCC，再按child-first顺序计算identity。
- boundary callable中的本Package named type直接使用Package schema ref；普通dependency与implicit std通过
  F162 exact bundle解析同一ref。删除HTTP结构特判作为boundary identity来源。
- `skiff.run/std`自身Package publication生成HTTP request/response/response-stream-event records；其它Package
  只能读取这些records。
- `compiler/contract`删除旧service-owned schema编译、复制与canonicalization；ServiceContract只选择Available
  operations，从resolved records计算精确传递闭包并生成排序的`PackageTypeRequirement`。
- 当前Package的resolved records由本次projection结果直接传给contract projection；依赖Package records使用
  F162 bundle。不得让contract crate访问filesystem。
- 删除或改写旧`ServiceContractDefinition.boundary_schema`产品路径，不保留兼容wire。

## 必须验证

- artifact-model、artifact-identity既有测试继续通过。
- compiler projection与contract crates恢复编译并通过聚焦测试。
- 真实源码PackageArtifact→schema records/index→ServiceContract。
- 两个service引用同一Package类型得到同一type id。
- 未引用公开类型变化不改变protocol；引用类型变化必改变protocol。
- Package version/build/service id变化不改变type id。
- std HTTP三类引用owner均为`skiff.run/std`，非官方同名拒绝，生产路径不再调用结构HTTP类型冒充身份。
- 未公开named child、缺dependency record、owner/key/id错配与SCC均fail closed。
- `git diff --check`；独立提交并写result，记录下一层consumer/runtime编译断面。

