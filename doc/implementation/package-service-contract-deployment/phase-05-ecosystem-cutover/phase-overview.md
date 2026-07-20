# Phase 05：Ecosystem Cutover

状态：planned；等待 P5-D01 独立文档评审

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
  RuntimeAssembly；request path不读artifact，不按service/version/build/display name选择旧路径。
- 外部ingress只使用canonical `(protocol, host, method, path)`；每个service有唯一Host。旧
  `X-Skiff-Service` / `X-Skiff-Version` 和 `?service=&version=` 不再有选择语义。
- 多个runtime replica加载同一完整assembly identity，每replica有独立heap/lifecycle，按
  deployment配置共享外部数据层；不承诺service级隔离或独立扩缩。
- `skiff-packages` 和 `internals` 的registry/platform、packages、contracts、deployments、actual
  services、clients 全部切换，provider/list 和 chat smoke 到达真实业务结果。
- 三个repo分别提交并合入各自 `main`；不push；所有已合并临时worktree/分支清理。

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
- 两个runtime replica注册相同assembly，Host ingress经router到provider得到业务结果；加载失败
  不替换旧active generation。
- Skiff、`skiff-packages`、`internals` 的production legacy 命中归零；fixture有replacement或删除证明。
- 完整non-live verify、隔离multi-replica动态probe和独立阶段验收在main merge前锚定同一
  冻结候选；三仓各自唯一main merge后，再按Internals规则运行stable registry/provider-list/chat
  验证。stable对象identity必须与冻结候选一致，每条昂贵证据只有一个owner。
