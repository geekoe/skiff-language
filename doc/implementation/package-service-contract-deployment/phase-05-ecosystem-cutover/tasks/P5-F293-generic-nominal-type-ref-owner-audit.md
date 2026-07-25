# P5-F293 Generic nominal TypeRef owner audit

状态：Ready。

## 直接父节点

- `P5-F292-generic-nominal-type-ref-gap-result.md`

启动时只读本任务；需要依据时沿父链向上读取。

## 任务

只读审计 fully-instantiated generic nominal type从 source到File IR、identity、linked type、
runtime value/catch identity的唯一生产与消费链，给出一个可直接实现的共享 checkpoint合同。

必须回答：

1. canonical `TypeRefIr` 应怎样表达 applied nominal：
   - 在哪些现有 nominal variant保存参数；
   - 或是否需要唯一新的 applied-nominal variant；
   - 如何严格禁止 primitive/container/interface/alias等非法 base；
   - 参数使用有序列表还是 declaration-param keyed map，以及怎样验证 arity/order/owner。
2. named-union已有 `ConcreteNominal.type_arguments`与 enclosing generic union如何避免重复、矛盾或丢失；
   ordinary generic nominal、generic representation、generic union branch、alias展开分别取什么 actual identity。
3. File IR schema/format、File IR identity、PackageArtifact build/local ABI、Package schema、ServiceProtocol中
   哪些必须变化、哪些必须保持；F288刚切换的新 generation是否需要再 bump。
4. 完整 production owner清单及最早不可逆损失，至少覆盖：
   - artifact-model DTO/strict serde/validator；
   - compiler core/source/lowering及所有 type traversal/rewrite；
   - artifact identity/cross-package rebinding；
   - runtime linker/linked-program/type plan/value carrier；
   - throw/catch/construct/pattern/container nested refs；
   - package/public schema generic支持或明确 fail-closed边界。
5. 能与正在运行的 F286无冲突并最短解除它的实现 DAG：
   - 最小共享 DTO checkpoint；
   - language consumer续接；
   - runtime consumer续接；
   - 机械 fixture/identity刷新owner。
6. 最小正负测试：同declaration不同args、nested args、generic representation、generic named union、
   cross-package symbol、missing/excess/unresolved arg、tampered wire、same-shape不同identity。

不得用 display/source text、shape、短名或 runtime address单独作为 type-argument identity；不得增加旧
artifact compatibility或双读。

## 只读范围与验证

可读取整个 Skiff repo与父链；禁止修改 production、test、reference或任务外文档。只新增一份：

`P5-F293-generic-nominal-type-ref-owner-audit-result.md`

无需运行 build/test；可用 `rg`、`cargo metadata --no-deps`和只读 git命令确认 owner。结果必须列出：

- 推荐唯一 shape及被否决备选；
- 精确 owner/写入范围与并行冲突；
- identity/version矩阵；
- 实现/验收 DAG和 focused命令；
- 是否存在需要用户决定的公共语义缺口。

## 风险与交付

- 风险：高，artifact identity/runtime catch identity共享 owner。
- worktree：`/Users/geek/workspace/skiff-p5-f293-generic-type-ref-audit`
- branch：`codex/p5-f293-generic-type-ref-audit`
- 不push、不操作stable。
- 启动到首次只读代码搜索不超过5分钟；完成后提交唯一result并返回commit。
