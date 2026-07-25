# P5-F306 Representation constructor carrier handoff audit结果

状态：Completed read-only。

## 首次损失

source resolution与expression checking均保留exact representation target、ordered generic arguments及
payload expression。`compiler/lowering/src/function_lowering.rs::lower_representation_constructor_call`
随后把target命名为`_erased_wrapper_type`并只返回payload ref。这是唯一首次不可逆损失。

File IR/linked IR没有可复用的typed wrap。现有`Construct`会分配record object，`InterfaceBox`会分配
interface carrier；借用它们或`throw.payload_type`会改变value shape或从static type猜actual identity。

## 唯一canonical handoff

```text
ExprIr::RepresentationWrap { value, type_ref }
LinkedExprIr::RepresentationWrap { value, type_ref }
```

- wire required `kind/value/typeRef`，无default、optional identity、display、fields或site；
- 求值child后保留原`RuntimeValue`，只把最外层carrier identity设为exact instantiated
  representation；
- target必须解析为plain/applied representation declaration；
- nested显式constructor产生nested wrap；不做隐式shape rewrap；
- named-union branch identity由目标union context按exact concrete nominal提升，不烘焙进wrap；
- wrap不是exception boundary，throw/call site不变。

## Generation

必须升级：

- File IR schema v7 → v8；
- File IR format v5 → v6；
- File IR identity prefix v7 → v8；
- opcode table保持v1；
- PackageArtifact schema与Local ABI/Build marker不再升版，File IR ref变化会自然改变build/program identity。

## DAG

```text
S0 artifact-model/identity shared DTO + generation
├── S1 compiler lowering producer
└── S2 linked-program/linker consumer
    └── S3 eval producer + exact nested/union promotion
(S1, S3) -> S4 combined probe/golden -> acceptance
```

Native return、boundary decode、call return、field/container projection与test effect producer均已由F299
覆盖，不能重复迁移。该DAG不需要新的用户语义决策。

