# P5-F296 Applied nominal compiler consumer

状态：Implemented checkpoint。结果见
`P5-F296-applied-nominal-compiler-consumer-result.md`。

## 直接父节点与权威链

- shared DTO acceptance：
  `P5-F295-applied-nominal-model-acceptance-result.md`
- language checkpoint：
  `P5-F286-open-error-language-source-lowering-result.md`

两份父结果继续引用F293 owner审计、open error设计与唯一权威语言/架构。启动时只读本任务；
需要依据时沿父链向上读取。

## DAG位置与共享状态

- 节点：F293 `S1 language/core/source/lowering continuation`。
- F286 non-generic CatchLeaves/declaration/site consumer已合入并通过core 41、source 306、lowering 46项。
- F295已冻结唯一 `AppliedNominal` wire与File IR/PackageArtifact新generation；不得改DTO、version或identity。
- 本任务完成后解除：
  - `S2 package/public fail-closed consumer`；
  - compiler/artifact combined probe与A2-language独立验收；
  - runtime local generic catch producer输入。
- 当前是实现检查点，不是稳定候选。

## Production写入范围

允许：

- `compiler/core/**`
- `compiler/source/**`
- `compiler/lowering/**`

只允许为下列目的修改：

- structured applied nominal producer；
- type-ref walk/rebind/substitute/closure；
- named-union branch与declaration instantiation；
- construct/pattern/throw/catch/call type argument lowering；
- F286既有检查适配新DTO。

允许上述owner的co-located tests，以及`compiler/tests/**`中本任务的source→File IR与package-direct
fixture。禁止修改artifact-model/identity、compiler compiled/projection-input/projection/contract/input、
deployment、test-runner、runtime、router、std、生态仓库或权威文档。

`compiler/source/src/callable_effects/**`只允许对`AppliedNominal.arguments`做穷举递归；
不得改变F278/F290 effect/provenance语义或断言。

## 完成标准

### 1. 唯一structured producer

- source name/type resolution在exact declaration owner与arity验证后直接产生：
  - 零参数nominal → existing plain ref；
  - 非零参数nominal → `AppliedNominal { base, ordered arguments }`。
- local、publication、service/package dependency owner全部保留exact locator与ABI expectation。
- 删除任何以`resolved_type_arg_texts`、source text、display、短名、suffix或shape参与语义恢复的路径；
  source text只可用于诊断。
- missing/excess/unresolved args、plain generic、applied non-generic、alias/interface/actor/DB非法base均在
  source/compile阶段fail closed。
- nested arguments递归保留；`TypeParam`只允许当前declaration/executable合法scope。

### 2. Type traversal与substitution

- core/source/lowering所有type-ref child/walk/path/closure/rewrite/assignability/substitution递归进入
  `arguments`并保留顺序；
- type closure按base declaration `type_params`与arguments zip建立substitution，不能用map作为artifact
  identity输入；
- generic representation保留外层nominal owner；transparent alias展开为target applied identity；
- PackageSymbol rehydrate继续保留F285 exact owner/PackageSchema return facts；
- actor/interface/conformance检查不能把AppliedNominal伪装成builtin/interface或bare address。

### 3. File IR lowering

- ordinary generic record/representation/union的signature、construct、pattern、container nested ref、
  throw payload、required catch type及call type args都写同一AppliedNominal；
- `NamedUnionBranchIr::ConcreteNominal`只写`nominal_type`，不恢复旧type argument map；
- generic named union declaration branch可包含`TypeParam`placeholder；实际`U<string>` usage/enclosing
  owner携带ordered string argument；
- source-authoredsite与F286required catch/site保持，不因适配generic丢失；
- File IR writer只产生F295冻结的v7/v5 shape。

### 4. CatchLeaves与非回归

- `Box<string>`、generic representation和fully-instantiated generic named union拥有确定leaves；
- 同declaration不同arguments、同shape不同owner、同branch不同enclosing union严格不同；
- anonymous union沿actual applied branch identity，不新增identity；
- unresolved/bare type parameter、nullable/mixed non-nominal仍按F286失败；
- 不增加schema/serialization要求，不开放public generic boundary；
- F286全部non-generic正负规则、F285 field-read与F290 effects测试继续通过。

## 最小测试矩阵与验证owner

至少覆盖：

1. `Box<string>`与`Box<number>` source→File IR不同ordered arguments；
2. nested `Outer<Box<string>, Array<Id>>`；
3. `type Token<T> = string`保留representation owner；
4. generic named union concrete/synthetic/literal branches与`U<string>`/`U<number>`；
5. local与cross-package同名generic owner不同；
6. alias到applied nominal；
7. missing/excess/unresolved args、plain generic、applied non-generic与非法base；
8. throw/catch/rethrow/pattern/construct/container nested ref；
9. F285 dependency callable field-read。

唯一owner：

```bash
cargo test -p skiff-compiler-core --lib -- --list
cargo test -p skiff-compiler-core --lib --no-fail-fast
cargo test -p skiff-compiler-source --lib -- --list
cargo test -p skiff-compiler-source --lib --no-fail-fast
cargo test -p skiff-compiler-lowering --lib -- --list
cargo test -p skiff-compiler-lowering --lib --no-fail-fast
cargo test -p skiff-compiler --test file_ir_execution_type_representation -- --list
cargo test -p skiff-compiler --test file_ir_execution_type_representation --no-fail-fast
cargo test -p skiff-compiler --test package_imports -- --list
cargo test -p skiff-compiler --test package_imports --no-fail-fast
git diff --check
```

先确认selector非零。若S2-owned projection或runtime loader仍在枚举前遮挡integration tests，记录精确首错，
不要修改范围外production。不得运行workspace、生态、stable、live或chat smoke。

## 风险与交付

- 风险：高；最终验收组仍为`A2-language`，本节点不单独宣称稳定候选。
- worktree：`/Users/geek/workspace/skiff-p5-f296-applied-nominal-compiler`
- branch：`codex/p5-f296-applied-nominal-compiler`
- 不push、不操作stable。
- 启动到第一次production修改不超过5分钟；不可执行时立即返回
  `TASK_NOT_EXECUTABLE`、精确缺口与最小前置。
- 提交后返回commit、反向搜索、自验收、遮挡与设计/owner缺口；不得自行承接S2/runtime/gate。
