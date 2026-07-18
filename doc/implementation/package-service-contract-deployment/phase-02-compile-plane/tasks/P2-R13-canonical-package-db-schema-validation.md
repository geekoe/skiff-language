# P2-R13：Canonical Package DB Schema Validation

## 背景

R10 审计发现，旧 service integration tests 中混有两类 DB 语义：collection mapping/Mongo
namespace 属于未来 ServiceDeployment/RuntimeAssembly；而 `_id`、index 与 field-path 校验属于
package source/schema compile。后一类不能因删除旧 service owner 而丢失。

## 目标

让 package compiler 的 canonical DB/schema owner 直接验证仍成立的逻辑 schema 规则，并以
PackageArtifact/File IR/runtime requirement 的 typed 事实证明结果。

## 边界

- 覆盖 `_id` 规则、空/重复 index、nested index/where field path 以及逻辑 DB schema declaration
  的现行语义；先核对 reference/source owner，无现行语义的旧断言直接删除。
- 允许修改 canonical package source/core/lowering/PackageArtifact DB projection 及直接 tests。
- 不读 service id、service.yml、collection mapping、Mongo physical namespace、deployment config 或
  serviceAssembly。这些事实延后给 ServiceDeployment/RuntimeAssembly owner。
- 不复活无 production caller 的旧 service storage validator，不建 compatibility wrapper。

## 完成态

1. 每个保留规则都由 production package compile path 调用，直接负例证明 fail closed。
2. 无 caller/dead validator 反向搜索为零；production/test 不引用 service publication/legacy DTO。
3. source/core/lowering 聚焦测试、compiler check、targeted rustfmt 与 `git diff --check` 通过，
   提交且 worktree clean。
