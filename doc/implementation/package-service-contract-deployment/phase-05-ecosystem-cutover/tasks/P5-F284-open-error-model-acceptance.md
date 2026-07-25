# P5-F284 Open error model A1 acceptance

状态：Ready after candidate assembly。

## 直接父节点与权威链

- 被验收实现任务：
  `P5-F281-open-error-channel-shared-model.md`
- 实现任务直接父结果：
  `P5-F280-open-service-error-channel-implementation-audit-result.md`
- F280继续引用F279及唯一权威设计。

启动时只读本任务，再读F281；只在需要判定语义时沿父链向上读取。

## 候选与验收边界

- production candidate commit：`a052f02a4e5d52c96d01849fa7df076f00df0d94`
- candidate tree：`4817f583de886d0b31e3fcf1141ac92532340bb0`
- acceptance worktree会把该production commit合入仅含task/result文档的integration HEAD；验收必须记录实际
  merge candidate HEAD，并确认相对`a052f02a`没有额外production diff。
- 风险/验收组：最高，`A1 strict-model`。
- 这是共享DTO冻结验收，不评审尚未迁移的compiler/runtime consumer，也不要求workspace可编译。

候选未冻结前已有开发证据：

- artifact-model 143/143；
- runtime-model 76/76；
- fmt与diff check通过。

这些证据可用于覆盖矩阵，但不得替代独立的canonical shape判断。

## 独立验收矩阵

### 1. 设计与owner

- File IR明确区分nominal record、representation、named union、transparent alias、interface；
- union concrete/discriminator/literal branch与enclosing context可表达稳定、互不混淆的identity输入；
- source-authored throw/call required site与显式synthetic site没有optional缺口；
- catch type required，旧`None`不能成为catch-all；
- runtime actual value/catch identity模型能区分local、Package schema、builtin、union context/branch与opaque cause；
- `ServiceErrorEnvelope`是唯一strict owner，三variant与F279/F280一致；
- generic control/internal `RuntimeErrorPayload`仍独立，未被字符串code全局替换。

### 2. 严格删除与版本

- model定义和serialized output不再含`throw_types`、`BoundaryErrorContract`、
  `BoundaryOperationContract.errors`；
- 旧`throwTypes/errors`、missing/null catch、missing source-owned site、unknown synthetic reason、
  malformed/extra envelope字段均被拒绝；
- 没有serde default、alias、legacy variant、dual read/write或display/shape fallback；
- File IR、format、PackageArtifact、ServiceContractDefinition与ServiceContract schema均只接受新version；
- artifact identity marker/prefix未在本节点偷偷bump，且结果明确留给唯一artifact/contract consumer。

### 3. Scope与下游handoff

- production diff只在F281授权owner内；其它`artifact-model/src/**`变化必须确实是co-located
  test/fixture constructor或删除旧字段所需的同crate窄consumer，不得改变无关生产职责；
- throw provenance、`throws_caller_alias`、`detached_error`及F278 same-heap事实未被删除或改义；
- 下游break清单覆盖language/lowering、artifact/contract、runtime identity/channel及机械fixtures，
  没有以兼容层隐藏遗漏；
- checkpoint足以让后续consumer只适配一次，不需要自行重新决定公共DTO。

## 独立探针

先审查实现与开发证据，再只运行足以验证高风险shape的聚焦命令。至少：

```bash
cargo test -p skiff-artifact-model --lib -- --list
cargo test -p skiff-runtime-model --lib -- --list
```

从真实列表选择并运行：

- artifact declaration/branch、required site/catch、old-field strict reject与schema version测试；
- runtime catch identity、nominal carrier、三variant envelope、opaque round-trip与malformed reject测试。

如少量目标测试无法共同过滤，可运行两个model crate完整unit suite；不得运行workspace/compiler/runtime完整
测试、`pnpm verify`、instance、live、生态publish或chat smoke。

执行：

```bash
git diff --check
```

不要机械重跑developer的fmt证据，除非审查发现格式状态已变化。

## 交付与禁止范围

新增并提交：

`P5-F284-open-error-model-acceptance-result.md`

结果必须记录：

- exact candidate HEAD/tree；
- 逐项矩阵与代码/test证据；
- `PASS`或`FAIL`；
- FAIL时的blocking finding、精确owner、最小修复范围与哪些开发证据失效；
- 是否存在新增用户设计决策。

除result文档外不修改任何文件；不修production、不改task、不中途扩成consumer评审。

- worktree：`/Users/geek/workspace/skiff-p5-f281-acceptance`
- branch：`codex/p5-f281-acceptance`
- 不push，不操作stable。一次性给出verdict后停止。
