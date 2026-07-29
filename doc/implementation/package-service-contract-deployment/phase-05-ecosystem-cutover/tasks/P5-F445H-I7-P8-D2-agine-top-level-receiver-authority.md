# P5-F445H I7 P8 D2 Agine topLevel receiver authority

状态：

```text
DOCS_ONLY
PRODUCTION_WRITE = NO
```

## 1. Parent and baseline

- 直接父节点：
  `P5-F445H-I7-P8-D1-package-direct-http-stream-task-refinement-result.md`建立的early diagnostic wave。
- 唯一顶层架构事实源：
  `../../../../architecture/package-service-contract-deployment.md`
- Skiff baseline：
  `2bcb40e61ee6b922eeca913651e2cc344a38b50e`
  （tree `df2bd49666a55d73f69b63b38c267bda8d2aed9d`）。
- 诊断用Internals candidate：
  `5861c13f3a92b7fb56a5cfa689e46f5d0462a02d`
  （tree `867c99c155386299e7dbb8b4fed95cee2427ba84`）。

## 2. Scope

本节点只把Agine默认测试的独立编译阻塞固化为最小权威语义与可执行A1任务。它不修复编译器，不运行
build/test/live/network/stable instance/Mongo/OAuth/browser，也不修改P8 package stream任务的根因、
证据或实现方向。

权威澄清是：

- `kind: test` service通过direct dependency的`topLevelAlias`取得精确implementation type后，可调用
  该type在同一精确artifact中已有的impl methods；
- source以exact `PackageSymbol`、`abiExpectation`和direct top-level view解析成既有
  `PackageCallable`；`AppliedNominal`保留相同owner/view和完整substitution；
- 源码显式参数不含receiver，lowering把receiver作为第一项执行参数；
- 普通public alias不自动公开任意impl method，service boundary对象不获得package-local methods，
  interface receiver仍走interface dispatch；
- 不新增语法、manifest/schema字段、artifact代际、关键字或运行时动态method lookup。

## 3. Completion

- D2 result记录可复现失败、三段缺口与非owner；
- A1拥有精确写集、RED/GREEN、negative与linked execution合同；
- DAG把P8 stream lane与Agine compiler lane分开，并在J前合流；
- J记录Agine当前为`170 declared / 0 discovered`，A1合流后才恢复最终170个默认测试；
- 文档反向检查不把本问题误写为Runtime、linker、test-runner或service boundary缺口。

执行结果：
`P5-F445H-I7-P8-D2-agine-top-level-receiver-authority-result.md`。
