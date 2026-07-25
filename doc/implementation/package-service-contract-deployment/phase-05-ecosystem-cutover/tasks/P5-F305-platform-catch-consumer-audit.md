# P5-F305 Platform catch identity consumer audit

状态：Ready。

## 直接父节点与权威链

- runtime local carrier结果：
  `P5-F299-runtime-local-exception-carrier-implementation-result.md`
- runtime owner审计：
  `P5-F280-open-service-error-channel-implementation-audit-result.md`

父链继续引用唯一权威error设计。

## 角色与边界

这是只读owner审计。F299删除旧`runtime/model::error::TypeIdentity`后，多个runtime crate仍在
`WirePayload::catch_projection`中消费旧类型。本任务只确定完整production范围、语义映射、依赖顺序与
leaf划分；不得修改/提交文件，不运行测试，不操作stable/live。

## 必须回答

1. 枚举所有production `TypeIdentity` import、返回类型、构造和比较；区分tests/fixtures。
2. 对每一处判断：
   - 是compiler/runtime-known platform catchable error，应映射到哪个exact
     `CatchIdentity::platform_builtin`；
   - 是ordinary diagnostic/opaque error，应保持`None`；
   - 是否误把未来service public typed error或`InternalError`当作string builtin。
3. 追踪`WirePayload` trait当前签名、所有转发wrapper与eval消费点，确认exact replacement是否会改变
   payload bytes、catchability、cancel/timeout选择或diagnostic wrapper。
4. 按crate依赖与写入范围拆成最少非重叠leaf，至少覆盖capability-context、native、
   linked-type-plan、service-db、request及受影响tests；标明哪些属于W2-R，哪些应留给W2-W。
5. 给出每个leaf的聚焦test命令、combined runtime-eval compile探针，以及上游失败遮挡关系。
6. 若存在需要新增公共身份、wire或用户决策的情况，明确指出；否则说明为何是既定模型的consumer迁移。

只允许`rg`、文件读取、Cargo metadata/manifest与git只读检查。返回精确`file:line`、完整矩阵与最短DAG；
不承接实现。

