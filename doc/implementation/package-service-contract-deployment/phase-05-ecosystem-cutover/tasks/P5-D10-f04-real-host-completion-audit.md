# P5-D10：F04 Real Host Completion Audit

## 角色与结论

R08 PASS后只读运行F04真实suite，审计为何仍没有`provider-observed-helper-mutated` Host结果。不得编辑、提交、
修复或给F04 verdict。

结论为`DESIGN GO`，存在两个独立F04完成态缺口：

1. isolated config输入已有`environment: skiff-test`，但`normalizeInstanceConfig`、`routerConfigText`与
   `renderRouterConfig`三层丢失该字段，Router因缺environment循环退出并最终readiness超时；
2. canonical source registry只有std，helper/service场景只在Rust临时fixture中project/assemble，从未publish、activate
   或走HTTP Host ingress。

## 冻结修复

- `local-instance-config.mjs`是environment唯一lexical owner：缺省`dev`，合法值匹配
  `^[A-Za-z0-9._-]{1,200}$`且拒绝`.`/`..`；normalize/summary保留字段。`skiff-instance`显式传config值，
  `skiff.mjs`传`dev`，`deploy-runtime-stack.mjs`传`prod`；renderer只做required保护与YAML输出。
- 保持source registry原样只有std。新增checked-in正常source fixture；现有package-service fixture binary增加互斥
  prepare-host-base模式，业务逻辑拆入聚焦Rust模块。
- preparer只调用production `build_authoring_object(..., publish=true)`依次发布contracts、helper/provider/consumer
  packages、provider/consumer deployments与base assembly，输出strict typed receipt；不得直接写store、patch/re-sign、
  手调mutation或创建ambient registry hook。
- source suite在std原样完成后显式prepare，严格验证receipt，再用现有runner的`--base-assembly`走overlay →
  `CanonicalTestRecords::publish` → activation → HTTP ingress。checked-in test精确断言最终值。
- 1092行Rust integration test改为复用同一fixture/preparer，删除重复source/writer/policy/loose JSON helper，避免
  project证据与real Host证据分叉。
