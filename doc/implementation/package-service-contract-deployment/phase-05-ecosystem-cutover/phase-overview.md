# Phase 05：Ecosystem Cutover

状态：active；P5-D01 已在 `838d909` / tree `617f159c` 独立评审 PASS；T01/F01/D02/F02已收敛，
P5-R01 在 `c168b1dc` / tree `961998ac` 独立复验 PASS，Wave 2 consumer已解锁

2026-07-28修正：I7真实组合assembly暴露旧Host全局selector设计错误。D0先冻结service-scoped ingress，
K及后续consumer再实现并重验；旧T03/F03B ingress证据不再有效。

2026-07-28修正：I7隔离验证还暴露Router把external request预算误用于assembly activation。P1D先冻结三个
互不替代的预算域：business request、activation prepare和WebSocket generation release；后续P1实现只按
`activation.prepareTimeoutMs`控制prepare事务，不能再从`requestTimeoutMs`或deployment policy取值。

2026-07-28修正：I7 M把Internals测试迁移为ordinary `kind: test` service后，Relay/Agine真实编译暴露
topLevel dependency的DB target仍依赖consumer复制/全图名字查找。P3D先冻结精确package symbol →
PackageBinding → provider File IR DB declaration链；P3实现必须让所有DB operation、`DbQuery`与lease路径
使用`DbObjectTargetId`，不得扩大普通dependency权限或复制provider metadata。

2026-07-28修正：AIHub测试既要使用subject公开API又要访问精确implementation顶层，旧
`access: topLevel`互斥模式表达错误；测试对transitive `llm-providers`的直接顶层访问还形成合法Package
diamond。D6硬切为同一entry的`alias` + test-only `topLevelAlias`；2026-07-30后same-build且owner facts相同
只保留一个metadata owner，数据库由generated test service identity派生，其余collection冲突继续失败。

2026-07-30修正：Phase 03/05把业务配置值、SecretRef和database state binding错误地装入
ServiceDeployment/RuntimeAssembly。F446按最新canonical架构硬切：一个service仍只有三层配置文件，文件根
直接按Package ID分区；配置进入独立RuntimeConfigSnapshot，activation generation并列钉assembly/snapshot
refs；一个service在受信storage domain/environment内只有一个系统派生数据库，删除全部author-authored
state/namespace与无效policy字段。

2026-07-30 F446收口复核：截至Skiff main `3344a535`，统一配置、activation exact-pair、service DB、
test-runner隔离及dead binding删除的实现checkpoint已经合流，但R446尚未开始。收口仍需删除
`collection_name_mapping`全链，以stable Package ID与declared logical collection identity系统编码physical
collection；RuntimeConfigSnapshot顶层增加受信target environment并在prepare/cold recovery物化
ConfigView前比较；数据库identity统一为operator选择的受信Mongo endpoint/storage domain +
environment + serviceId，不引入platformId；Router↔Runtime frame统一为v3。当前状态和证据边界见
[`P5-F446-closure-result.md`](tasks/P5-F446-closure-result.md)，不得把implementation checkpoint写成R446
PASS。

## 输入

- PackageArtifact、ServiceContract、ServiceDeployment、RuntimeAssembly 和完整 InProcessBoundary 生产路径。
- Phase 04 最终 production candidate `13b4600f` / tree `a34e103c`，已经由 merge commit
  `5c3322ac` 合入 `main`。
- Phase 04 动态交接的 `registrations=0` 和 Agine `/session` fail-closed：它们是本阶段必须
  转成真实正例的端到端缺口。

## 完成态

- strict authoring 输入、immutable storage、release pointer 和 environment activation state 只产生/
  消费四个canonical对象；没有共同aggregate、legacy adapter、dual read/write、fallback。
- contract 可先于implementation发布；package只依赖已发布ServiceContract独立编译；deployment
  只用typed artifacts校验；assembly解析完整闭包并原子activation。
- router、runtime、CLI/watch/dev sync、test-runner、package-test 与fixtures全部切到active
  RuntimeAssembly；request path不读artifact，不按build/display name、query、rewrite或HTTP Host选择
  deployment。
- 外部ingress按Host等平台规则注入可信`x-skiff-service`/`x-skiff-version`。Router严格解析这两个header，
  先选active assembly中的唯一精确deployment，再在该deployment内按canonical
  `(protocol, method?, path)`选择entry；不同service可共享相同method/path，同service重复失败。
- 多个runtime replica加载同一完整assembly identity，每replica有独立heap/lifecycle，按
  deployment配置共享外部数据层；不承诺service级隔离或独立扩缩。
- external business request、activation prepare与WebSocket generation release各自拥有独立预算；
  assembly activation不会因service/request timeout到期而被abort。
- `skiff-packages` 和 `internals` 的registry/platform、packages、contracts、deployments、actual
  services、clients 全部切换，provider/list 和 chat smoke 到达真实业务结果。
- 三个repo分别提交并合入各自 `main`；不push；所有已合并临时worktree/分支清理。
- `kind: test` service可在direct dependency entry设置`topLevelAlias`，同时保留普通`alias`访问公开API；
  顶层权限不传递，consumer不复制DB metadata，linked/runtime按artifact+file+type index精确选择。
- 同一stateful package的direct/transitive diamond在exact build与完整resolved mapping canonical相同时只
  激活一个collection projection/metadata owner；其它mapping/build/root冲突失败。

## 实现批次

1. 在 Skiff integration branch 建立 authoring/storage/control 共享检查点，独立验收后扇出
   tooling、router、runtime 和test-runner。
2. 合流Skiff生产consumer并建立可供外部repo消费的exact checkpoint。
3. 从该exact checkpoint执行Skiff terminal legacy deletion，并行迁移 `skiff-packages` 与
   `internals`；Internals 先冻结code-free contracts，再将registry/platform、Codex Relay/AIHub、
   Agine/clients分为非重叠owner，最后由单一owner形成包含全部actual deployments的environment assembly。

详细DAG、ownership、risk probe、候选成熟度与gate owner见 `phase-plan.md`。

## 阶段验收

- production source tree 不存在四对象之外的共同aggregate、旧DTO/reader/writer/converter、
  dual path、request-time artifact load或runtime fallback。
- 平台真实支持contract-first publish、package independent compile、deployment validation、
  complete assembly activation、prepare/commit/abort CAS、stale generation fail-closed和pre-commit reject rollback。
- 两个runtime replica注册相同assembly；ingress注入精确service/version header，Router选deployment后
  按method/path到provider得到业务结果；加载失败
  不替换旧active generation。
- Skiff、`skiff-packages`、`internals` 的production legacy 命中归零；fixture有replacement或删除证明。
- 完整non-live verify、隔离multi-replica动态probe和独立阶段验收在main merge前锚定同一
  冻结候选；三仓各自唯一main merge后，再按Internals规则运行stable registry/provider-list/chat
  验证。stable对象identity必须与冻结候选一致，每条昂贵证据只有一个owner。
