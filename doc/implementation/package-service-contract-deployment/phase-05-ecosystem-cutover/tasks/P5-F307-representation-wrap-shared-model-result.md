# P5-F307 Representation wrap shared model结果

状态：Implemented，等待F308。

任务提交：`bd44c1b038ba48eb5a0f5288baa2234d174fec7a`。

集成提交：`3dbd2119f6899d781e8068d6a529f3a7d3c6a932`。

## 结果

- 新增唯一strict `ExprIr::RepresentationWrap { value, type_ref }`；
- wire required `kind: representationWrap`、`value`、`typeRef`，missing/null/legacy alias/附加字段拒绝；
- visitor递归进入完整type arguments，admission验证child存在；
- target仅接受plain/applied exact Representation declaration，arity/nested args完整验证；
- record/union/alias/interface/primitive、unresolved owner、残留TypeParam与PackageSchema全部fail closed；
- owner、nested argument或child ref变化均改变File IR identity。

## Generation

- File IR schema v8、format v6、identity prefix v8；
- opcode v1保持；
- PackageArtifact v5、Local ABI v3/v5、Build v4/v6、ServiceProtocol v4保持。

## 验证

- artifact-model list/full：PASS，156/156；
- artifact-identity list/full：PASS，94/94；
- `git diff --check`：PASS。

F308 PASS前不解除compiler/linked consumers。

