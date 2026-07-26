# P5-F379 OpenAI nullable-union audit

状态：Ready（只读）。

## 直接父节点

- `P5-F375-registry-generation-revalidation-result.md`

父节点的fresh full skiff-packages gate在发布`openai`时失败：

```text
openai/openai.skiff:465:62
openai/openai.skiff:471:58
readImageResponse argument 3 canonical type identity/type mismatch
expected union containing a nullable member, found nullable union
```

本节点先判定这是OpenAI source call shape写错、类型别名声明不一致，还是compiler canonical
nullable/union assignability错误；不在审计中修改production。

## 审计要求

1. 用fresh canonical std和当前Skiff toolchain精确复现`openai` publish failure。
2. 沿`readImageResponse`声明、两个caller、`input.outputFormat`声明及其alias/import逐跳列出：
   - source spelling；
   - resolved canonical type；
   - nullable位于union内还是union外；
   - 哪一跳出现预期/实际identity分叉。
3. 检查skiff-packages中相同类型与相同nullable-union写法，判断影响面是否只限两个caller。
4. 对照语言reference和现有compiler测试，确定唯一owner：
   - source应显式重排/解包；或
   - compiler应把语义等价的nullable-union规范化为同一identity；或
   - 二者语义本就不同，应给出应使用的准确source类型。
5. 给出最小production/test文件、正负例、focused命令与是否需要用户决策。

## 边界与交付

- 审计Skiff与skiff-packages均只读；
- 不改OpenAI source、compiler、测试或manifest；
- 不运行stable/live或外部OpenAI请求；
- 临时artifact/store必须隔离并清理；
- 不派子Agent。

在本任务Skiff worktree写
`P5-F379-openai-nullable-union-audit-result.md`并本地commit。若唯一修复路径已由现有语言规则确定，返回
`TASK_EXECUTABLE`；只有确需改变语言语义时才返回`TASK_NOT_EXECUTABLE`及精确用户决策题。
