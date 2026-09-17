# Changelog

本文件记录 `crypto-arb-rust` 的主要变更。版本号使用 [SemVer](https://semver.org/lang/zh-CN/)；
Unreleased 区段汇总尚未发版的改动。

## [Unreleased]

### Changed

- **确定性机会工件**：机会预览新增快照、票据、幂等身份、Checksum 与 TTL 绑定的执行工件；
  对冲执行把复制只读校验、重新验证和人工提交拆成独立动作，Paper/Shadow 可继续自动闭环，
  Live 自动化只生成工件和提醒，不替操作员提交订单。
- **Webhook Provider 与确认语义**：新增 Bark JSON 投递、device-key 脱敏和官方应用层
  `code=200` 确认；手机提醒包含双腿路径、收益、成本、证据、有效期、失效条件和只读校验命令，
  并过滤候选扫描心跳等高频噪声。通用签名 Webhook 的 HTTP 2xx 明确标为仅传输成功。
- **Open Design 工作流 UI**：机会扫描和自动化增加紧凑 Webhook 健康带；自动化收起低频配置、
  突出最新工件和决策账本；费后净差、止盈和止损阈值统一以百分比输入和展示。

## [2.2.0-rc.2] - 2026-07-28

### Highlights

- **自动套利运行安全**：策略关闭或环境切换会立即清除 Live 解锁；真实后端运行环境、
  全局 Kill Switch 和至少一项止盈、止损或强平保护共同构成入场硬门禁。
- **自动化实时状态**：新增 `automation` AppWS 频道、replay 与断线降级回退，前端不再依赖
  高频轮询显示自动策略状态，并补充杠杆、并发和冷却配置的可见性。
- **链上 / CEX 观察增强**：Jupiter `/swap/v2/order` 支持无 API key 的官方低频只读额度，
  严格校验金额、精度、新鲜度与成本参数，并单独展示 CEX 费率、滑点、Gas 和原始数量。
- **Webhook 可靠性加固**：业务事件 ID 在重放和重启后保持稳定；仅对临时 HTTP 状态重试，
  并扩大 CGNAT、文档网段、基准测试网段和 IPv4-mapped IPv6 的 SSRF 拒绝范围。
- **前端体积门禁复核**：相对 `rc.1` 的 gzip 增量约 6.2 KB；三阶段回归上限按当前功能面
  重新评审并保留约 2% 余量，`1.2 MB` 产品优化目标不变。

### Release boundary

- 这是候选版；自动套利仍默认关闭，Live 自动化必须在每次进程启动后显式解锁。
- 自动 Paper 闭环验证脚本已加入，但只有市场存在满足费后净收益门槛的真实候选时才会自动开仓；
  当前验证环境没有生成符合条件的候选，因此未把真实行情自动成交闭环标为已通过。
- 无交易所私有凭证时无法完成真实小额 place/cancel/finality 验收；链上比较始终保持只读，
  不构造、不签名也不广播链上交易。

## [2.2.0-rc.1] - 2026-07-28

### Highlights

- **自动化套利候选链路**：新增默认关闭的自动化控制器、机会筛选、暂停和紧急停止，
  模拟与实盘均复用 HedgeTicket 预览、确认、ExecutionRun 和自动盈利平仓证据链；
  实盘模式必须在每次进程启动后输入显式短语解锁，并继续受全局 kill switch 约束。
- **Solana 链上 / CEX 价差监控**：接入 Jupiter Swap V2 只读 order 报价与现有 CEX
  盘口缓存，展示双向毛价差、手续费、滑点、gas、流动性、新鲜度和净价差；不传
  `taker`，不请求交易，不签名也不广播。
- **安全 Webhook**：覆盖机会、自动化决策、执行结果、补偿、风险告警和系统降级事件，
  提供 HTTPS 限制、DNS/IP SSRF 防护、HMAC-SHA256 签名、幂等事件 ID、有界队列、
  超时、有限重试和最近投递诊断。
- **API 与前端工作台**：新增自动化、链上监控和 Webhook 的共享 DTO、API 路由、
  后台生命周期、设置面板与响应式样式；对冲确认逻辑从 router 下沉到 service。

### Release boundary

- 这是候选版；三项新能力仍在产品审计队列中，默认不会自动执行实盘交易。
- Jupiter 读取需要 `JUPITER_API_KEY`；Webhook 密钥仅保存在当前运行时内存中，重启后
  需要重新填写。
- 无交易所私有凭证时只能验收公共行情、模拟链路和配置界面；真实下单、成交终态及
  外部 Webhook 目标仍需在部署环境中完成小额验证。

## [2.0.0] - 2026-07-21

### Highlights

- **CROSSLINE Omni 工作台重构**：产品收敛为持仓/风控、期货套利、机会扫描、对冲执行、
  复盘和设置六个主模块，并以共享 DTO、真实运行态和 typed problem 作为前后端事实边界。
- **三类 P0 套利策略**：正式锁定 `PerpCross`、`SpotPerp` 与 `CrossSpotPerp`；机会列表、
  HedgeTicket、ExecutionRun、ActionRun、CloseRun 和复盘账本形成完整可追溯链路。
- **八家交易所证据合同**：Binance、OKX、Bybit、Bitget、Gate、HTX、KuCoin 与
  Hyperliquid 的 adapter/parser/request fixture、instrument registry、公开行情、私有读取、
  下单/撤单和终态边界均由 registry 与非跳过门禁锁定。
- **运行安全与故障可见性**：Bearer/WS ticket、非 loopback 启动保护、凭证安全存储、
  高风险动作审计、幂等提交、部分成功、unwind、降级 envelope、source/freshness/retry/problem
  均进入产品界面，不再用零值或乐观 ACK 伪装成功。
- **产品与性能验收**：本地可执行路线图 182/182 完成，2020/2020 个审计目标文件具备 exact
  evidence；机会热路径使用轻量分页 DTO、增量 WS patch 和浏览器/Wasm/二进制预算门禁。

### Release boundary

- 默认适合 Paper 模式与无凭证公共行情体验；Live 模式只校验当前 HedgeTicket 涉及的交易所。
- 真实小额 place/cancel/finality 与 configured private order stream capture 仍需用户提供凭证完成
  PR-M / PR-FR 外部验收；本地 fixture 不会伪造这些实盘证据。

### Added

- **资金费率矩阵视图**（前端 `views/funding_rates.rs`）：按 symbol × exchange 透视，
  颜色映射绝对值大小（绿正 / 红负 / 5 级强度），支持搜索、30s 自动刷新、统计 strip、
  i18n 双语 toast。
- **后端 WebSocket 实时推送套利**（`crates/api/src/lifecycle.rs`、`api/ws.rs`）：snapshot
  updater 每轮完成后向 `WsHub` 的 `arbitrage` 频道广播；前端 `views/arbitrage.rs` 内置
  `start_arbitrage_stream` 客户端，自动重连 + 25s 心跳，连接状态在工具栏 `WsStatusBadge`
  显示（Connected / Connecting / Disconnected）。
- **全量中英 i18n**（`frontend/src/i18n.rs`）：覆盖 nav / kpi / table / tooltip / errors / toast /
  ws / funding 全部 UI 文本，`tr(lang, key)` 单点查询。
- **全局 Toast 系统**（`state/mod.rs`、`components/ui/toast.rs`）：4 级（success / error /
  warning / info）+ 5s 自动消失 + 手动关闭，全部视图（套利、Funding、模拟、期权、Chat）
  统一接入。
- **每行陈旧高亮**（套利表）：基于 `now_ms` 全局 1s tick 与 `updated_at` 时间戳，30s+
  弱化 / 60s+ 琥珀左边框 + 工具提示，配合 KPI strip 的 freshness 颜色分级。

### Changed

- **OKX 适配器优化**（`crates/exchange/src/adapters/okx.rs`）：默认基址从 `www.okx.com`
  切到 `app.okx.com`（部分网络环境下前者被中间设备深度干扰，funding-rate 端点频繁
  超时）；批量场景新增 `fetch_funding_rate_one_fast` 快路径，绕过 retry/backoff，单条
  失败丢弃；`FUNDING_RATE_CONCURRENCY` 8 → 32，`OkxConfig::default().qps` 10 → 25。8
  家交易所现可在 30s 内全部覆盖。
- **Backpack/BP 下线**：移除运行时 adapter、签名模块、API Key 配置、探测脚本、聚合器
  测试和前端设置入口；当前执行 venue 集合收敛为 8 家。
- **聚合器单家超时**（`crates/exchange/src/aggregator.rs`）：`fetch_all_funding_rates` 加
  `PER_EXCHANGE_TIMEOUT = 30s`，慢于此的交易所被丢弃但不阻塞整体；与 snapshot updater
  的 30s interval 对齐。
- **snapshot updater 重构**（`crates/api/src/lifecycle.rs`）：合并独立的 prewarm 任务到
  主 loop，单 task 串行运行；`tokio::time::interval` 改为
  `MissedTickBehavior::Skip`，避免 scan 慢时连续无停顿触发交易所限流；每轮加
  `scan_ms / publish_ms / count` 结构化日志，超 `interval × 1.5` 自动 WARN。
- **统一深色主题**：移除 light theme 全部分支与 localStorage 持久化逻辑，
  `frontend/index.html` 硬编码 `dark` class，避免主题切换闪烁；UI 组件
  （Input / Select / Button / Badge）清理浅色 utility classes。
- **AppState arbitrage_snapshot interval 20s → 30s**（`crates/api/src/state.rs`），与
  aggregator 单家超时一致，OKX 全量覆盖时不会被打断。

### Performance

- 单次 `/api/arbitrage/funding-rates` 端到端：3499 条费率 × 9 venues × 982 symbols，
  ~25s 稳定（OKX qps=25 + fast 路径 + app.okx.com）。
- 套利 snapshot 一轮 16–21s（8 家并发），WS 推送间隔 ≤ 35s（30s interval + 串行 scan）。

### CI / Ops

- GitHub Actions：新增 `frontend` job —— 装 wasm32 target + Node 20 + Trunk + binaryen，
  跑 `cargo clippy --target wasm32-unknown-unknown` + `trunk build --release` +
  `wasm-opt --Oz` 体积报告（写入 GITHUB_STEP_SUMMARY），3 MiB 硬上限防膨胀。
- `fmt` job 拓展为 backend workspace + frontend 两段 check。
- `release-build` job 依赖更新为 `[fmt, clippy, test, frontend]`。

---

## [1.0.0] – 早期里程碑（参考 git 历史）

- **M1–M13**：从 `common`、`domain`、`exchange`、`arbitrage`、`options`、`llm`、
  `realtime`、`simulation`、`trading`、`api` 到前端骨架 + 5 个业务视图，最终带 Docker
  Compose 三服务发布。
- 8 家交易所适配器（Binance / OKX / Bybit / Bitget / Gate / HTX / KuCoin / Hyperliquid）
  + 套利 V3 引擎（6 算法）+ Black-Scholes 期权引擎 + 4 LLM 路由 + 实盘 / 模拟下单。
- Hyperliquid 风格 UI（mint / bull / bear / ink palette），KPI strip，详情面板（4 sections，
  含费率矩阵 / 成本分解 / 仓位大小计算 / 风险警告）。
