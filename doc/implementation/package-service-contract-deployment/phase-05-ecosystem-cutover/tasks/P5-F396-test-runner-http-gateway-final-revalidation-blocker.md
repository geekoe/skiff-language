# P5-F396 Test-runner HTTP gateway final revalidation blocker

状态：TASK_NOT_EXECUTABLE（前置cherry-pick顺序缺F390；无production变化）。

F396从clean F387 checkpoint直接cherry-pick F392时，在：

- `router/tests/compilerGeneratedManifestCompatibility.test.ts`
- `router/tests/dynamic-build-id-parity.test.ts`

发生内容冲突。原因不是F387拥有Router改动；F387相对base的owned diff中`router/**`为零。F392的exact
current-record测试建立在F390先迁移compiler fixture与这两个compatibility tests之上。

Agent已`cherry-pick --abort`，branch仍为clean
`71687e3765fc302611aad5de22a095d1621e4b8f`，无新commit。正确依赖顺序为：

```text
F390 53c79dc6 -> F392 e4cf2431 -> F394 540f93c4 -> F396 gates
```
