# P2-R09：Canonical Test Dependency Closure

状态：absorbed；canonical PackageArtifact/package graph 部分并入 P2-R10，旧 runtime witness 提交不移植，
不再单独执行。

## 目标

compiler test-support 必须保留 production pipeline 产生的 `PackageArtifact`、File IR 与同轮
`CanonicalPackageGraph`，使 fixture 能覆盖编译后引入的 implicit std，而不从 source imports
建立第二套依赖规则。

## 完成态

1. compiler test-support typed 结果只保留 canonical `PackageArtifact`、精确 File IR 与
   `CanonicalPackageGraph`，不包含 `PackageUnit`、`ServiceUnit`、runtime unit 或空兼容槽位。
2. graph 选择复用 compiler source facts/production package resolution，使实际冻结 File IR 引用的
   implicit std 有机会在同轮 graph 中编译；不从 test-runner imports 表推导，不硬加
   std，不要求用户为 test assert 手写 std import。
3. ordinary、显式 std import 与 assert rewrite 的 implicit std 均走同一 canonical graph 路径；
   File IR 未引用 std 时不伪造 requirement。
4. 不修改 canonical wire/identity/source/lowering 语义，不二次 compile，不复制 R06 closure 算法。

## 写入范围

- 具体写入由 P2-R10 接管：`compiler/driver/test_support/**`、integration fixtures 与直接测试。
- 只能复用 canonical package graph resolver/source facts；禁止保留、改名或调用旧
  `resolve_service_packages` service resolver。不得修改 source/lowering 语义、canonical artifact wire、
  test-runner 或 runtime。

## 验证

- P2-R10 聚焦测试覆盖显式 dependency、implicit std from frozen File IR 与 unused std。
- 反向搜索证明 test-support 无 `runtime_units`、旧 artifact holder 或 import-based 第二规则。
- 完整 compiler gate 仍由 P2-T07 执行一次。
