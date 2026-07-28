# P5-F445H-I7-D7 Stateful package diamond

状态：`READY_FOR_INTEGRATION`。

## 1. Parent authority and baseline

- authority：`P5-F445H-I7-D6-test-alias-diamond-authority.md`；
- historical implementation input：`P5-F415-collection-mapping-current-integration-result.md`；
- observed ecosystem blocker：`P5-F445H-I7-M2-shared-helper-publish-order-result.md`；
- Skiff baseline：`5e87d1ce3c3461e5687564807afea9db4943ba46` /
  `c9481fc7859919199ac84e6839b07847779fce02`。

D7只实现D6已经冻结的stateful package diamond admission。它不修改manifest/compiler、P3 foreign DB target
consumer、Internals或外部package。

## 2. Required implementation

1. dependency graph保留direct和transitive两条真实edge，并继续只为exact `PackageBuildId`分配一个code
   slot；
2. artifact-model提供非序列化的canonical runtime comparison helper；它比较完整resolved source→target
   collection map和activation database namespace，exact build继续隐含拥有不可变DB metadata facts；
3. loader与Host使用同一helper：
   - exact build且canonical projection相同：合并为一个effective projection和一个metadata owner；
   - exact build但canonical projection不同：fail closed；
   - different build指向同一physical target：fail closed；
   - dependency与root collection冲突：fail closed；
4. identical edge不能重复插入target、metadata或code slot，也不能简单忽略第二条edge而不比较；
5. edge顺序、reload与committed recovery不得改变结果；
6. 不升级artifact、identity或runtime wire generation，不增加兼容reader。

## 3. Tests and ownership

- artifact-model：empty/explicit identity canonical equivalence、mapping/namespace difference；
- loader/Host：AIHub形状 `test root -> C` 与 `test root -> subject B -> C`，覆盖empty identical、
  non-empty identical、不同edge顺序、same-build different mapping及既有distinct-build/root collision；
- test-runner：保留两条真实incoming links，同时只生成一个exact provider code slot；
- 运行artifact-model、runtime-loader、runtime-host和test-runner相关locked suites，以及check/fmt/diff/rg。

若实现需要新增公开artifact DTO、改变schema generation，或不能从exact build、resolved map与activation
namespace判断canonical equality，返回`TASK_SCOPE_EXPANDED`。

