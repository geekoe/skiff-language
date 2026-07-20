# P5-D01：Phase 05 独立文档评审

## 角色与输入

未参与Phase 05计划编写的只读评审Agent。完整阅读：

- 权威设计 `doc/architecture/package-service-contract-deployment.md` §1–§15；
- 本阶段 `phase-overview.md`、`phase-plan.md` 及全部 `tasks/P5-*.md`；
- `doc/implementation/package-service-contract-deployment/AGENTS.md`；
- `/Users/geek/workspace/multi-agent-development.md`；
- Phase 04 `phase-result.md` §6、§8–§9。

不得修改文件、创建commit、运行完整gate或用兼容层填补DAG。

## 必验问题

1. T01是否只冻结authoring/storage/control共享检查点，不创造第五对象/common kind、
   不迁移consumer；R01是否能独立验证strict wire、CAS、resolver及cross-language fixture。
2. T02–T05是否从R01 exact checkpoint真实扇出，写入范围不重叠；tooling、router、runtime、
   test infrastructure是否各自包含旧consumer删除条件。
3. Host-only ingress、single active assembly、full-assembly replica registration、prepare/commit/abort与
   pre-commit rollback是否
   与设计一致，没有偷渡service/version/build selector或RemoteBoundary。
4. T06是否只在R02后删除legacy model/reader/writer；checker是否覆盖真实production subjects、
   omission/rename/move/duplicate/test-only camouflage，而不是简单字符串白名单。
5. 外部repo是否从exact Skiff checkpoint开始；Internals contracts、shared workflow、Codex、AIHub、
   Agine、registry/platform是否遵守contract-first依赖及非重叠owner。
6. AIHub/Agine的package-local nominal types是否明确要求contract-owned schema + explicit wrapper；
   state/config/secret binding、account/registry service、provider/list正例是否没有遗漏。
7. 三个执行批次、候选成熟度、combined probe、gate preflight、唯一昂贵owner、最终
   合入/清理是否可执行；是否有无法验收或重复证据。

## 输出

第一行必须是 `PASS` 或 `FAIL`。`FAIL` 列出blocking issue、设计/任务证据、影响与
建议owner；另列non-blocking improvement。`PASS` 仍需列出已检查DAG、ownership、风险probe、
gate与残余风险。
