# P5-F178A：Actor Declaration Artifact Checkpoint Result

状态：Completed

## 直接父任务

- `P5-F178A-actor-declaration-artifact-checkpoint.md`

## 交付

- syntax新增显式`actor Name id IdType { fields }`声明及独立`ActorDecl` AST；精确保留actor name、
  id type和有序field shape。
- parser禁止generic actor、缺失id、重复actor、actor与普通type同名，并严格拒绝
  `type ... implements Actor<Id>`伪造actor；既有`native type`拒绝保持不变。
- compiler input-model新增`ActorDeclarationInput`/`ActorFieldInput`，从已解析AST提供bootstrap专用
  field shape；compiled新增带module path的`CompiledActorDeclaration`只读事实与accessor，不执行
  actor typing或lowering。
- artifact-model新增独立`ActorDeclarationIr`与`ActorAbiInput`，ABI输入覆盖actor name、id type、
  有序field layout、逐field canonical encoding、公开方法签名及actor runtime ABI version。
- actor声明/ABI事实与既有`ActorMetadataIr`保持分离：前者描述source declaration、bootstrap shape
  和ABI，后者继续只描述Runtime执行路由；本任务未改Runtime metadata wire。
- Actor ABI严格wire拒绝legacy `ActorRef`、未知runtime ABI version、重复field和重复public method。
- artifact-identity新增canonical `actor_abi_identity`入口，identity随id type、field layout/type、
  public method输入和runtime ABI version变化。
- 为使checkpoint相关crate越过F177公共硬切，只机械删除compiler内残留
  `TypeDescriptorIr::Native`穷举分支和native-type provider读取路径；interface descriptor使用既有
  空record占位。没有实现actor source checking、lowering、std native API或Runtime执行。

## 验证

通过：

```text
cargo test -p skiff-syntax -p skiff-artifact-model \
  -p skiff-artifact-identity -p skiff-compiler-input-model
# syntax: 109 passed
# artifact-model: 112 passed
# artifact-identity: 74 passed
# artifact-identity CLI: 8 passed
# compiler-input-model: 2 passed

cargo test -p skiff-compiler-compiled --lib
# 3 passed

git diff --check
```

`cargo check --workspace`已越过全部compiler crate，首错位于
`runtime/linker/src/linker/file_conversion.rs`对已删除`TypeDescriptorIr::Native`的消费。该错误属于
后续Runtime迁移，本任务按范围未修改。
