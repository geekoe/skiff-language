# P5-F391 Inferred suspension contract design

状态：Ready（用户已确认；文档先行）。

## 直接父节点

- `P5-F382-interface-suspension-projection-audit-result.md`
- 用户决策：不新增`async`/`suspending`/effect关键字，不要求interface提供“绝不挂起”保证。

## 冻结语义

1. 有函数体的concrete executable继续从body、调用图及内建等待点推断`maySuspend`；开发者不重复声明。
2. package公开concrete callable保留推断出的effect summary，供依赖package编译和传播。
3. interface method requirement不包含`maySuspend`，interface conformance不比较该位。
4. 通过未知interface/`any I`动态调用时保守视为可能挂起；已知concrete/public-instance binding使用具体
   callable的推断summary。
5. service call本身就是调用方的挂起点。callee实现内部是否还会挂起不是ServiceContract协议承诺，不应
   改变ServiceProtocol identity。
6. runtime仍只在stream next、异步service call、timer等真实等待点释放actor；没有显式`yield`，声明或
   保守分析不会主动切换执行。
7. 若Host/runtime需要callee实际summary来选择内部执行或取消机制，它属于implementation/deployment
   metadata，不属于interface或ServiceContract identity。

## 文档写入

至少校正：

- `doc/reference/interface.md`
  - 删除“interface完整signature/effect conformance包含suspension”的旧规则；
  - 说明dynamic dispatch保守分析和concrete binding推断。
- `doc/reference/static-semantics.md`或现有effect/suspension直接owner
  - 明确推断、跨package summary传播和调用分类。
- `doc/architecture/actor-model.md`
  - 只补充“保守maySuspend不等于实际让出”，不得恢复yield。
- `doc/architecture/package-service-contract-deployment.md`
  - interface requirement facts不含suspension；
  - concrete callable summary仍是Package ABI fact；
  - ServiceContract不携带callee internal suspension；
  - deployment/runtime metadata边界。

沿这些文件的直接引用同步修正明显冲突的规范段落，但不要重写无关设计。

## 必须说明的兼容/identity结果

- concrete package callable从non-suspending变成suspending仍会改变其Package Local ABI/build，并使依赖
  package重编译；
- interface requirement本身不因实现effect改变Local ABI；
- ServiceContract operation request/response/公开错误不变时，callee internal effect改变不应改变
  ServiceProtocol identity；
- implementation/deployment/build identity仍可随实现变化；
- operation/callable stable ID不受影响。

## 交付

文档任务，不修改production/test/artifact schema。写
`P5-F391-inferred-suspension-contract-design-result.md`，并给后继实现审计列出所有需要删除/迁移的旧字段与
identity golden类别。

本地commit、worktree clean；不merge/rebase/push，不派子Agent。若文档间出现用户尚未决定的另一项语义，
停止并精确上报，不自行扩展。
