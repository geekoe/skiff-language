# P5-F411A Alias-return-catch-once subject API control file

状态：Ready。

## 直接父节点

- `P5-F411-runtime-router-test-fixture-generation-sync-result.md`

F408–F411 合流后的聚焦验证已经通过。主 Agent 随后在精确 Skiff integration
`b6d2cdfee0e21a485f33a1e29c1b1f8490a3a887` 上运行完整
`node scripts/run-skiff-tests.mjs`：isolated instance 正常启动、canonical std 11/11
通过，随后 source suite 在进入测试执行前因以下唯一清单缺口 fail closed：

```text
test-runner/fixtures/alias-return-catch-once/api.yml: api.yml is required
```

对 `test-runner/fixtures/**/package.yml` 根执行完整控制文件审计，只发现
`alias-return-catch-once` 缺失 `api.yml`；它是被测试 Package subject，不声明公开面。

## DAG位置与范围

- 节点：F411 合流后的源码套件机械修复。
- production start：`b6d2cdfee0e21a485f33a1e29c1b1f8490a3a887`。
- 只允许新增：

```text
test-runner/fixtures/alias-return-catch-once/api.yml
本任务result
```

`api.yml` 内容必须是严格的空 mapping：

```yaml
{}
```

不得修改 parser、test-runner production、其它 fixture、测试语义、设计或生态仓库。若加空控制文件后
聚焦 source-suite test 暴露另一个独立 blocker，只记录精确失败并返回；不得吞并新节点。

## 验证与交付

至少运行：

```bash
node --test scripts/tests/skiff-source-test-suite.test.mjs
git diff --check
```

另用只读脚本或 shell 审计每个 `test-runner/fixtures/**/package.yml` 的同目录 `api.yml` 均存在，记录根数与
缺失数。不要运行完整 isolated suite、stable/live 或外部服务；不得派子 Agent。

写 `P5-F411A-alias-subject-api-control-file-result.md`，提交代码和 result，返回 exact commit/tree、
测试计数与清单计数。保持 worktree clean；不 merge/rebase/push。
