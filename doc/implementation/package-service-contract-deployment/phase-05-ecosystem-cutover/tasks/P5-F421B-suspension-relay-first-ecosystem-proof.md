# P5-F421B Suspension Relay-first fresh ecosystem proof

状态：Ready（F421任务合同修正后的N5 successor）。

## 直接父节点

- `P5-F421-suspension-relay-first-ecosystem-proof.md`
- `P5-F421-suspension-relay-first-ecosystem-proof-result.md`

F421没有执行任何publish或assembly；唯一失败是任务合同把“Skiff production冻结锚点”和“包含task
文档的checkout HEAD”写成了同一个精确值。本节点只修正这个执行边界，不改变F421任何生态范围、
语义、命令、验证、写入权限或PASS标准。

## 修正后的精确输入规则

### Skiff

- production/executable冻结锚点继续是：
  `9f39580655ecbd433235cdb7de19d823d670d4a9` /
  `d20cd4ccd8f11042a1f4bc6dac69d3ccda1116b9`；
- 本任务base精确是：
  `bc23b84850155045c5e08532186466e243ebf536` /
  `ac39e5cd2b41c0b64f19b1fe43cebd5c8ad33765`；
- 本任务checkout的parent必须精确为本任务base；checkout相对parent只能新增本任务文件；
- `9f395806...`必须是checkout的ancestor；
- 从production冻结锚点到checkout，排除
  `doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/**`
  后必须零diff；
- Skiff integration root允许精确位于本任务checkout，因为它包含已调度的task/result文档；**不得**
  再要求integration HEAD/tree等于production冻结锚点。

### 另外两个repo

- Internals必须精确为：
  `baf0c907ee26e48a5fb4c153825c233bde3a6234` /
  `13f2f6e604fedbad80e0390e5408507430e28f8c`；
- skiff-packages必须精确为：
  `0972e65604cd4cfd45bcdb289cfe5019f57dc265` /
  `1849f97a1f1217b95e6e349bc529eaaf220a62f4`。

三个production tree仍必须clean且无并发production writer。只有违反上述修正规则才是input mismatch。
F421已经记录并接受的task-only Skiff提交不是漂移，不能再次据此停止。

## 执行与交付

除上述input gate替换外，完整执行
`P5-F421-suspension-relay-first-ecosystem-proof.md`中的：

- 写入、环境与角色边界；
- Gate前置预检；
- 唯一fresh rebuild及Relay-first顺序；
- Relay exact verdict；
- 全生态pair/callback/mapping/consumer重算；
- canonical负例与反向搜索；
- receipt、失败收敛与最终verdict。

唯一tracked写入改为：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
P5-F421B-suspension-relay-first-ecosystem-proof-result.md
```

若真实production incompatibility出现，按F421规则收集同一sibling wave后提交
`TASK_SCOPE_EXPANDED / N5_FAIL`；不得修改source。全部条款通过才提交
`N5_PASS / PHASE_05_ECOSYSTEM_PROOF_COMPLETE`。

这是新的单次gate-owner会话，不复用F421 Agent。不得merge/rebase/push/stable/live。
