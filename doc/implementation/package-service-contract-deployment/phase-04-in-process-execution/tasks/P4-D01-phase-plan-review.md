# P4-D01：Phase 04 独立文档评审

## 角色与输入

未参与Phase 04计划编写的只读评审Agent。完整阅读：

- `doc/architecture/package-service-contract-deployment.md` §2、§6–§10、§12、§14、§15；
- 本阶段`phase-overview.md`、`phase-plan.md`与全部`tasks/P4-*.md`；
- `doc/implementation/package-service-contract-deployment/AGENTS.md`；
- `/Users/geek/workspace/multi-agent-development.md`。

不得修改文件、创建commit或用Phase 05 authoring/tooling替本阶段补洞。

## 必验问题

1. 三波DAG是否形成短shared kernel后真实扇出；T01/T02、T04–T06、T07–T09写入范围是否可并行且没有中央文件争抢。
2. `AssemblyExecutionImage`是否仍只链接code一次，package direct与service call是否保持不同语义；是否避免
   `ServiceUnit`/`PackageUnit`/legacy `EvalRuntimeProgram` adapter。
3. ActivationContext、request generation、binding vector、materializer、callback table owner和lifetime是否严格符合设计。
4. ordinary/error、async/stream/cancel、callback/native三个lane是否均有可执行正负例；Phase 02 unsupported source
   lane的边界是否被正确留给Phase 05，而没有用内部手写target绕过typed production pipeline。
5. ingress/internal是否进入同一dispatcher；host active-generation pin与router service-relay retirement是否完整，
   又没有误删gateway、actor/spawn控制语义。
6. 每个高风险边界是否有独立验收、最早探针、唯一gate owner和精确证据失效范围。
7. Phase 04/05边界是否清晰；是否偷做authoring/registry/release或新增RemoteBoundary/compatibility fallback。

## 输出

第一行必须是`PASS`或`FAIL`。`FAIL`列出blocking issue、设计证据、任务证据、影响和建议owner；另列
non-blocking improvement。`PASS`仍需列出已检查的DAG、ownership、gate和残余风险。
