# P5-D12：Runtime Committed Bootstrap Audit

## 角色与结论

R10 PASS后恢复F04A真实Host probe，health已出现capability connection与Router exact committed generation-0
snapshot，但replicas始终为空。D12只读审计cold startup/reconnect、admission与registration；不得编辑、提交、修复
或给F04 verdict。

结论为`DESIGN GO`：production Runtime从空`AssemblyAdmissionState`启动，配置没有environment与singular canonical
artifact root，也没有durable `EnvironmentActivationState` reader；连接时因此只发送capabilities，无法发送committed
register。新activation又只能从healthy committed replicas冻结participants，形成启动僵局。

这是F03C已有Runtime startup/admission/lifecycle职责的DAG排序遗漏。冻结F10/R11提前拆出committed recovery与
reconnect sync；Router正确地区分capability与admitted replica，不得让F09/F04A把capability直接升级成participant。

## 冻结恢复语义

- Runtime config要求exact environment与singular canonical artifact root；拒绝missing/empty、旧plural与多root，不扫描
  environment、不选latest。
- 每次连接Router前都从exact environment path读取完整activation state，只恢复`committed`；验证schema、path/
  environment、assembly ref/content identity后执行完整resolve/load/link/validate/admit。
- committed recovery在一个admission state-lock transaction中同时发布local active context与committed tuple，然后才
  连接并按`runtime.capabilities → assembly.activation/register`发送。generation 0也走同一primitive。
- durable committed是已完成Router CAS的恢复凭证，不伪造activationId/prepared/commit。pending绝不自行激活；先注册
  committed，再由Router按原transaction重放prepare。
- reconnect前丢弃非durable staged heap state并重新同步committed，防止离线期间generation前进后循环stale register。
- online prepare/commit与cold recovery复用exact resolve/admit及committed publication primitive；删除或限制可绕过
  transaction的production direct-admit入口。request trust boundary仍留给R05后的F03C。
