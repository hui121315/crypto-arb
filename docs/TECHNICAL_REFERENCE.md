# CROSSLINE 技术与运维说明

本文保留原 README 的接口、配置、运行和验证说明；本次整理不改变技术实现。
产品怎么用见 [README](../README.md)，操作逻辑见 [产品逻辑](./PRODUCT_LOGIC.md)。

## 交易所接入

Gate CrossEx 是带底层场所身份的执行路由，不是独立流动性来源，因此不能与它所指向的同一家
直连交易所组成套利双腿。

| 接入 | 公开行情 | 私有状态 | 生产写入 |
|---|---|---|---|
| Binance | USD-M / Spot WS | 账户与订单 WS | Spot 与 USD-M WS API |
| OKX | V5 Public WS | V5 Private WS | Private WS |
| Bybit | V5 Public WS | V5 Private WS | V5 Trade WS |
| Bitget | UTA V3 Public WS | UTA V3 Private WS | UTA V3 Trade WS |
| Gate | Spot / Futures WS | Spot / Futures Private WS | Spot / Futures WS API |
| Gate CrossEx | Dedicated CrossEx WS | Dedicated CrossEx Private WS | CrossEx Private WS |
| Kraken | Spot v2 + Derivatives WS | Spot v2 + Derivatives Private WS | Spot v2 WS；Derivatives REST v3 |
| KuCoin | Spot / Futures WS | Classic / Pro Private WS | Spot Pro WS；Futures Classic REST |
| Hyperliquid | `l2Book` / `activeAssetCtx` / WS info | 账户、订单与成交 WS | WS `post` |

逐 venue、逐 operation 的当前 API 家族、保留 REST 原因和官方文档见
[交易所传输矩阵](./EXCHANGE_TRANSPORT_MATRIX.md)。Kraken 与 Gate CrossEx 的独立凭证、
流量和运行证据见 [专项接入说明](./KRAKEN_GATE_CROSSEX_WS_INTEGRATION.md)。

## WS / REST 边界

浏览器 AppWS 与交易所 WS 是两层连接：交易所事件先进入后端缓存和账本，后端再把紧凑增量推送给
Leptos 前端。AppWS 大帧使用 `zlib-json` 压缩，小型 ACK、心跳和低延迟事件保持文本帧。

| 数据 | 热路径 | REST 保留职责 |
|---|---|---|
| Ticker、Funding、mark/index | 公共 WS | 冷启动、首帧等待、断档恢复和周期校准 |
| 执行盘口 | 被选候选与构建/提交前按需 WS | 首帧或序列缺口时有界恢复 |
| InstrumentSpec | WS 更新或官方 HTTP registry | 精度、最小数量、挂牌、费率与慢变元数据 |
| 余额、持仓、挂单 | 私有 WS 热缓存 | 初始快照、分页和 scoped reconciliation |
| 订单与成交终态 | 私有 WS | 身份点查、历史、费用补齐和结果不明恢复 |
| Funding payment 与历史 | 本地账本 + signed REST | 历史事实不从实时流反推 |

REST 不是实时行情的轮询替代品。WS 没有首帧、断线或出现序列缺口时，恢复请求会单飞并有界退避；
存在旧快照时明确标为 stale，没有旧快照时保留原始错误。

Funding 的全市场 REST 轮次只负责发现候选；周期历史、REST API、AppWS 重放和策略读取都从
同一个行情缓存投影。候选 WS 行会覆盖发现基线，聚合状态不会因为一个未配置的可选场所而把
已有新鲜行整体标记为不可用，具体来源与问题保留在逐行证据和场所明细中。

## 性能与流量

性能优化的目标是删除无价值的连接和重复请求，而不是降低有效行情的实时性：

- 扫描器只为初筛费后净收益为正的候选维护有界 ticker/Funding 订阅，不为每个币建立独立 WS。
- 机会扫描不订阅全市场盘口；选中候选可预热两腿，构建与提交前才严格读取深度。
- 相同市场的选择、预览和提交共享单飞缓存，避免连续操作重复请求。
- Binance 深度订阅控制帧按 500ms 批处理，行情事件仍保持官方 100ms 节奏。
- AppWS 以不低于 100ms 的批次发送市场、持仓和系统增量；订单与风险事件立即发送。
- 私有账户 REST 恢复按 venue 单飞退避，前端刷新不会放大成请求风暴。
- Portfolio REST、AppWS、自动退出和系统健康读取同一份生命周期快照；HTTP 请求不会重新拉取
  账户、历史或风险。生命周期停摆时返回带新鲜度的 `Stale` 快照，冷启动则返回明确的
  `Unavailable`，不会按客户端数量重复计算。
- SystemHealth REST 同样只读 5 秒生命周期快照；首帧前返回 `Warming`，状态栏请求不会即时
  重建诊断、读取账户或查询 PnL 历史。

Kraken Derivatives Funding 扫描必须使用包含 Funding、mark 和 index 的完整 ticker；当前专项样本中，
274 个可执行永续约产生 `3.5 MB / 10s` 接收流量，但只使用一个 Futures WS，且不订阅全市场盘口。

## 快速开始

### 环境要求

| 工具 | 要求 |
|---|---|
| Rust | `1.95.0`，由 `rust-toolchain.toml` 固定 |
| Wasm target | `wasm32-unknown-unknown` |
| Trunk | `0.21.14` |
| Node.js | `20` |
| 常用工具 | `curl`、`jq`、`python3`、`rg` |

### 安装与启动

```bash
git clone --branch codex/product-plan-execution \
  https://github.com/hui121315/crypto-arb.git
cd crypto-arb

rustup target add wasm32-unknown-unknown
cargo install --locked trunk --version 0.21.14
npm ci

npm run dev:restart
```

启动完成后打开：

- 工作台：<http://127.0.0.1:8080>
- API：<http://127.0.0.1:8000>
- Readiness：<http://127.0.0.1:8000/health/ready>

公共行情和 Paper 不要求交易所凭证。首次使用不需要一次配置所有交易所。

### 本地进程

```bash
npm run dev:up       # 端口空闲时启动
npm run dev:restart  # 清理项目拥有的旧 listener 后重启
npm run dev:down     # 停止已记录的本地进程
bash scripts/verify_runtime.sh
```

运行快照与日志位于：

```text
.crossline-runtime/dev/runtime_snapshot.json
.crossline-runtime/dev/logs/api.log
.crossline-runtime/dev/logs/frontend.log
```

遇到白屏、数据不刷新或端口冲突时，先检查这里的 PID、日志和 runtime snapshot，不要把残留进程
误判为产品数据故障。

页面加载失败现在有独立恢复入口：样式、脚本或 WASM 未就绪时保留可读提示，不显示白屏或无样式交易面板。
等待超过 12 秒只提示加载较慢，不自动刷新；迟到的资源仍可正常完成启动。手动“重新加载”保留当前 URL
和当前标签页的待核验记录，不清除站点数据、不自动重发业务请求。页面未打开不代表后台任务已停止。
加载完成后的账户/API 错误仍由各模块处理，不会被启动提示遮住或误报为资源故障。
实现使用 [Trunk 的启动模板接口](https://github.com/trunk-rs/trunk/blob/v0.21.14/src/pipelines/rust/output.rs)，保留原有哈希资源和预加载。
局部复核：构建前端后执行 `CROSSLINE_E2E_PROFILE=release npx playwright test test/e2e/startup-recovery.spec.ts --workers=1`。
该测试以受控 HTTP/WS 注入资源失败和保存中刷新，不证明真实部署网络稳定性或全部交易入口的刷新恢复。

### 平仓结果恢复

单仓、配对和全部平仓共用待处理状态。提交中切页不解除锁；当前标签页刷新后保留原请求编号，
“核验原平仓”只查询原回执，不重新下单。记录按后端地址和登录上下文隔离，不存凭证或下单正文。
订单受理、待补偿、未知结果和最终成交分别显示；只有匹配原请求及目标的有效终态才解除待处理。
账户读取失败不隐藏核验入口，成交回执也不直接删除持仓，实际仓位仍由账户快照确认。

后端明确在提交前拒绝的快照/腿数/参数错误显示“未提交”，无需把它当成未知成交一直锁住。
双腿中任何一腿结果未知，不因为另一腿已拒绝就允许重复平仓。关闭标签页、清除站点数据、
原账本过期与跨客户端并发不在此保证范围内。

补偿下单、补偿撤单及人工终结同样保存原请求编号，支持切页和当前标签页刷新后的只读核验。
补偿与撤单使用独立的待处理记录：不能重复补单，但已确认的补偿订单仍可撤销。
核验匹配原动作及其嵌套补偿订单/人工记录，不把历史上另一笔成功当成本次成功。
撤单接口成功只代表已受理；另查后端本地订单记录或接收关联 WS 终态后才显示已取消。
这次查询不触发交易所账户重拉，不自动重发撤单。人工终结只记录处理结果，不代表系统已平仓或盈利。
技术详情默认折叠；账户暂不可读时仍可核验原请求。未提交表单的草稿不保证刷新后保留。

部分成交的补偿单可以撤销未成交部分，已成交数量保留；页面分别显示目标、已成交与未完成数量，
数量未知时显示“待确认”，不算成零。只有后端确认上一笔零成交且仍有重试额度时，才提供带新请求标识的
补偿重试入口；已有成交或成交量未知时禁止按原数量整笔重试，需要核对实际仓位后处理剩余风险。
撤单后保留事故记录及精确复盘链接，不把原事故敞口当成已重新核算的实时剩余敞口。
账户 NAV 缺证据不会连带把新收到的本地平仓记录判为过期；实际刷新失败仍禁止基于旧记录提交。
补偿回执建立前已经成交时，后端按原订单补读有界成交账本，只有完整数量匹配才补回确认时间，
不重复累加成交量或费用。实盘下单 ACK 不能替代成交证据；证据不足时整笔补偿仍保持待确认。

### Rust 缓存维护

启动前、停止后、成功完成 `npm run finish` 后自动检查项目的两个 Cargo `target` 目录；
不安装常驻进程，不扫描整个用户目录，也不会停掉正在运行的服务来清缓存。

| 空闲检查点的状态 | 处理 |
|---|---|
| 合计低于 16 GiB | 保留热缓存，避免反复重新编译 |
| 达到 16 GiB | 删除 7 天未更新的增量编译条目，保留依赖和可执行文件 |
| 达到 32 GiB，或达到 16 GiB 且磁盘空闲不足 20 GiB | 先清增量；仍超过软限时，用 Cargo 清理较大的 debug/test 构建目录 |
| 正在构建、Trunk 运行或后端运行 | 延后清理，不杀进程、不删除使用中的文件 |

这些是**空闲检查点阈值，不是构建过程中的实时磁盘配额**。自动清理保留 release、自定义 profile、
前端 dist、源码、配置、数据库、Git、全局 Cargo/Rust 工具链缓存和废纸篓。
若 release/自定义构建目录本身超过上限，会告警而不是擅自删除。
手动使用原生 `cargo` 不会触发项目检查；长时间直接构建后运行 `npm run cache:guard`。
外部 `CARGO_TARGET_DIR`、Cargo 自定义 build-dir 和其他工作区不在本机制的管理范围内。

```bash
npm run cache:report   # 查看项目缓存和阈值
npm run cache:preview  # 只查看自动清理计划，不删除产物
npm run cache:guard    # 执行分级清理
npm run cache:trim     # 只清增量缓存，保留已编译依赖与程序
npm run cache:clean    # 明确清空两个 target（含 release）及生成的前端产物
npm run cache:test     # Python 3.11+，在临时目录验证清理机制，不构建产品
```

阈值可通过 `PROJECT_CLEAN_THRESHOLD_GIB`、`PROJECT_CLEAN_HARD_LIMIT_GIB`、
`PROJECT_CLEAN_MIN_FREE_GIB`、`PROJECT_CACHE_STALE_DAYS` 调整。前后端测试 profile 禁用增量缓存，
日常 dev 仍保留增量编译及错误行号。深度清理后的首次编译会变慢，不承诺固定缓存缩减比例。
外部 `CARGO_INCREMENTAL` 或 Cargo 全局配置仍可覆盖 profile；报告会提示已设置的相关环境变量。
只在已核实的 Cargo 缓存目录内清理，拒绝符号链接入口和 Git 跟踪文件；进程状态无法读取时不删除。
检查与手动启动原生 Cargo 并非原子操作，维护期间不要另开终端同时启动构建。
原来的全局 `--pressure-clean` / `--clean-dev-caches` 不再提供，避免项目维护波及其他工具。

依据：[Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)、
[cargo clean](https://doc.rust-lang.org/cargo/commands/cargo-clean.html)。

### 推荐验收顺序

1. **Paper**：不配置凭证，完成候选、预览、双腿终态、持仓和 CloseRun 闭环。
2. **账户只读**：逐家配置最小权限凭证，确认余额、持仓、挂单和私有 WS。
3. **Shadow**：使用真实主网行情和账户证据检查规格、费用、Funding、深度与延迟。
4. **Live 预检**：保护既有人工仓位，确认 Kill Switch、限额、余额和撤单/终态能力。
5. **小额 Live**：只对当前票据执行，确认双腿成交、残仓归零和真实净收益后再扩大范围。

## 凭证配置

凭证可以在“设置 → API 凭证”中保存，也可以从 [`.env.example`](../.env.example) 创建本地 `.env`。
桌面长期运行推荐使用 macOS Keychain：

```bash
APP_CREDENTIALS__SECRET_BACKEND=keychain
```

| 接入 | 必填字段 |
|---|---|
| Binance | `BINANCE_API_KEY`, `BINANCE_API_SECRET` |
| OKX | `OKX_API_KEY`, `OKX_API_SECRET`, `OKX_PASSPHRASE` |
| Bybit | `BYBIT_API_KEY`, `BYBIT_API_SECRET` |
| Bitget | `BITGET_API_KEY`, `BITGET_API_SECRET`, `BITGET_PASSPHRASE` |
| Gate | `GATE_API_KEY`, `GATE_API_SECRET` |
| Gate CrossEx | `GATE_CROSSEX_API_KEY`, `GATE_CROSSEX_API_SECRET` |
| Kraken Spot | `KRAKEN_SPOT_API_KEY`, `KRAKEN_SPOT_API_SECRET` |
| Kraken Futures | `KRAKEN_FUTURES_API_KEY`, `KRAKEN_FUTURES_API_SECRET` |
| KuCoin | `KUCOIN_API_KEY`, `KUCOIN_API_SECRET`, `KUCOIN_PASSPHRASE` |
| Hyperliquid | `HYPERLIQUID_ACCOUNT_ADDRESS`, `HYPERLIQUID_PRIVATE_KEY` |

Hyperliquid 的 Account Address 是真实交易账户地址，Private Key 是 API/Agent wallet 私钥；
不要把 Agent wallet 地址填成 Account Address。

链上 Provider 可选配置：

| Provider | 字段 |
|---|---|
| Jupiter API Key | `JUPITER_API_KEY` |
| 0x | `ZEROX_API_KEY` |
| OKX DEX | `OKX_DEX_API_KEY`, `OKX_DEX_SECRET_KEY`, `OKX_DEX_PASSPHRASE` |
| CoW | 公共 Fast Quote 无必填 Key |
| Solana 钱包签名器 | `ONCHAIN_SOLANA_PRIVATE_KEY`（Base58 或 32/64 字节 JSON keypair） |
| EVM 钱包签名器 | `ONCHAIN_EVM_PRIVATE_KEY`（32 字节 hex 私钥） |

钱包私钥在保存时会推导地址并与当前钱包严格比对，状态接口不会回显密钥。Solana 可使用 Jupiter
API Key 托管发送或已核验的自定义 RPC；EVM 提交必须使用与所选链 ID 一致的自定义 RPC。

不要给交易 API key 开启提现权限。优先使用独立子账户、最小读取/交易权限和 IP 白名单。

## 运行诊断

```bash
# 服务和依赖任务
curl -fsS http://127.0.0.1:8000/health/ready | jq

# 当前环境、adapter、Kill Switch 与限额
curl -fsS http://127.0.0.1:8000/api/trading/status | jq

# 账户、持仓与风险快照
curl -fsS http://127.0.0.1:8000/api/trading/portfolio/snapshot | jq

# 当前分页机会
curl -fsS \
  'http://127.0.0.1:8000/api/v3/arbitrage/opportunities/list?pageSize=20&fast=true' | jq

# 实际 REST / WS operation 注册表
curl -fsS http://127.0.0.1:8000/api/trading/transport/registry | jq
```

顶部状态栏中的字段表示不同层级：

| 字段 | 含义 |
|---|---|
| `MarketData N/M` | 已启用的核心公开行情 operation 可用数；主动停用项不计入分母 |
| `TradingAPI` | 已配置场所的私有读取与写入 operation 状态 |
| `PrivateWS` | 交易所账户/订单私有流，不是浏览器 AppWS |
| `AppWS` | 浏览器是否完成后端频道订阅 |
| `订单终态` | 订单创建到确认终态的耗时，不是网络 RTT |
| `Risk / Delta / Funding` | 账户与结算证据；未配置或缺样本时保持未知 |
| `Mode` | 当前 Paper 或 Live 环境 |

顶部系统摘要同时读取快照内容与读取状态。首读失败不显示正常；后续失败可以保留上次数值，
但标明“系统数据待确认”，Risk、Delta、Funding 与订单耗时同时携带错误提示。
已有风险警告或阻断不会被刷新失败清除；恢复后才解除待确认提示。慢 HTTP 回包不能覆盖
请求发出之后收到的 WS 状态。此规则不改变行情更新频率，也不代表真实账户已完成验收。
对应隔离浏览器路径：`test/e2e/system-health-state.spec.ts`。

系统快照、运行健康和交易状态均绑定发起读取时的后端与 Token。切换连接清空旧值及旧退避，
立即读取新来源；旧连接的挂起请求会取消，切回同一地址也不接收上一轮回包。每类共享读取
最多一笔在途，15 秒未返回会取消并报错，不累积挂起请求；正常周期仍为 5 秒，系统快照优先 WS。
快照超过 15 秒未更新会标明待确认，重复时间戳、倒退快照或电脑时间回拨不能延长有效期；
只在同一连接保留旧数值，新证据到达后恢复。交易模式与风控尚未确认时，顶部不显示运行正常。
对应受控浏览器路径：`test/e2e/shared-state-freshness.spec.ts`，不代替真实后台长期运行验证。

共享 WS 在切换后端或 Token 时清空旧来源的频道错误、计数和帧时间；连接地址与鉴权票据
固定属于同一次连接。每次物理重连使用新代次，旧关闭回调不能再发起第二条连接。
同源重连保留错误历史，但必须收到新帧才重新计算数据时效，订阅 ACK 不代表行情已恢复。
格式错误的消息不再误报物理断线；服务端鉴权拒绝仍保留阻断行为。
对应隔离路径：`test/e2e/ws-source-recovery.spec.ts`，不代替各模块业务缓存或真实网络长期验证。

期货套利与机会扫描的共享候选、搜索、分页和详情也绑定当前连接。切换后端或 Token 时，
保留用户的币种/策略筛选，但清空旧报价、分页游标和选中详情；重新等待当前来源的数据，
不会把等待显示成“没有机会”。过时读取会取消，A→B→A 的迟到回包也不能填回旧值。
两页的读取错误统一在页内和顶部显示，不再重复弹出遮挡控件的浮动报错。
对应受控路径：`test/e2e/opportunity-source.spec.ts`；有独立有效搜索快照时仍可进入预检，
没有当前来源的报价时不能用旧候选构建。此验证不包含实盘成交。

对冲预检同时绑定当前执行环境、适配器与风控配置。配置变化或状态失效后，旧票据与勾选确认
立即失效，输入金额保留；恢复后重新预检、校验和确认。切回原模式也不能接收上一轮迟到结果。
后端预检模式与当前设置不一致时拒绝生成执行凭据；急停或实盘写入关闭时不构建新订单。
复用既有共享状态读取，不增加轮询；错误保留在页内，不重复弹出遮挡操作的浮层。
对应受控路径：`test/e2e/execution-preview-context.spec.ts`。这不替代服务端提交时的账户、风控与订单校验。

提交结果未知时，执行路径、订单区与详细状态统一显示“原提交待核验”，不把没查到记录当作未下单。
原运行单与可选拒单回执查询共用 15 秒上限；超时取消本次读取、保留原提交身份，重试只查询原单。
页面刷新后查到的明确下单前拒绝仍保留可见结果，只有匹配原请求的证据才能解除恢复锁。
对应受控路径：`test/e2e/execution-recovery-read.spec.ts`，不代替真实交易所成交及跨客户端验收。

运行记录已有订单编号而最近 50 笔快照未覆盖时，订单区显示已知订单数和明细覆盖数，
按原编号补读本地订单账本，不把缺失明细当成 0 单或成交失败。每轮最多并发两笔、共用
15 秒上限，失败可手动补读缺项；重复运行帧不触发重复查询，切页取消读取，旧回包不覆盖
更新的 WS 记录。编号错配的回包不采用，不补造委托或成交数据。
对应受控路径：`test/e2e/execution-order-coverage.spec.ts`；补读不是交易所重新查单或重新提交。

从复盘查看原执行时，流程摘要改为历史记录的四个阶段，不再提示选择新机会；查询只匹配
原运行、票据与机会编号，找不到则保持待确认。订单受理不当作成交，执行收口也不直接等于
双腿均已平仓：单腿失败后的补偿收口须查看原订单与补偿记录。打开新票据后，上次成交与
回执明确标注为历史；原执行未收口仍保留防重复提交保护。
对应受控路径：`test/e2e/execution-history-flow.spec.ts`，覆盖刷新和精确往返，不证明真实资金终态。

顶部摘要与行情、交易接口、私有账户流、后台任务和应用连接使用同一套状态判断。
没有样本不显示运行正常；未配置、等待样本、数据过期显示中性待确认，降级显示黄色，
明确读取失败或接口受限才显示红色。模拟模式明确未配置的私有账户不算故障，也不代表实盘就绪。
展开状态详情可直接查看具体原因，无需悬停；已有风险警告和阻断仍优先显示。
对应受控浏览器路径：`test/e2e/status-readiness.spec.ts`。此判断针对后台实际提供的状态行，
不证明所有预期交易所、品种或接口均已覆盖。

`Degraded` 不表示整个系统不可用。应展开问题并按 `venue + operation + source + requestId + retry`
定位。常见状态包括：

- **构建时读取深度**：扫描阶段没有批量读取盘口，点击构建后才核验双腿。
- **缺交易所挂牌证据或可执行规格**：InstrumentSpec 尚未确认挂牌、精度或最小数量。
- **账户数据待配置**：缺少 API key 或读取权限，不是公共行情故障。
- **私有 WS 等待事件**：连接和订阅可能已就绪，但尚未出现真实账户或订单事件。

## 安全与部署

默认只绑定 `127.0.0.1`。绑定到 `0.0.0.0` 或其他非 loopback 地址时，后端要求 Bearer、
显式 CORS 和审计日志，否则拒绝启动：

```bash
APP_HOST=0.0.0.0
APP_SECURITY__AUTH_TOKEN=change-me
APP_SECURITY__ALLOWED_ORIGINS='["http://127.0.0.1:8080"]'
APP_SECURITY__AUDIT_LOG_PATH=/var/lib/crossline/security_audit.jsonl
```

Docker Compose：

```bash
APP_SECURITY__AUTH_TOKEN=change-me \
APP_SECURITY__ALLOWED_ORIGINS='["http://127.0.0.1:8080"]' \
docker compose -f deploy/docker-compose.yml up -d --build

docker compose -f deploy/docker-compose.yml ps
docker compose -f deploy/docker-compose.yml down
```

高风险动作、凭证变更和交易提交都会保留脱敏审计与 request ID。运行数据默认写入系统应用数据目录；
API key 不写入执行账本、AppWS 或前端状态。

## 开发与验证

日常界面和流程变更优先使用少量定向 E2E；已通过且未受改动影响的检查不重复运行，不默认扩跑几十项单元检查。

```bash
# 查看本批次将运行哪些检查
npm run finish:plan

# 日常变更：按受影响后端、前端、UI 或文档执行一次检查
npm run finish

# 发布范围需要时选用：工作区级强验收、前端 release build 与 Wasm 预算
npm run finish:release
```

浏览器模拟闭环使用独立配置，按本次改动选择一条用户路径：

```bash
# 前端代码有变化时先更新 frontend/dist
(cd frontend && trunk build --offline)
# 手动构建 → 双腿提交 → 持仓 → 配对平仓 → 关联复盘
node node_modules/@playwright/test/cli.js test --config test/e2e/paper-cycle.config.ts --grep "paper opportunity opens a pair"
# 或：后台已成交但回包丢失 + WS 断开 → 刷新页面 → 只读找回原单 → 平仓 → 原记录复盘
node node_modules/@playwright/test/cli.js test --config test/e2e/paper-cycle.config.ts --grep "paper opportunity recovers a lost submit reply"
# 或：保存保护/入场规则 → 自动开仓 → 暂停 → 手动退出 → 回执和复盘
node node_modules/@playwright/test/cli.js test --config test/e2e/paper-cycle.config.ts --grep "paper automation"
# 或：自动开仓 → 暂停新入场 → 后台止盈/止损退出 → 原运行复盘
node node_modules/@playwright/test/cli.js test --config test/e2e/paper-cycle.config.ts --grep "paper protection"
# 或：缺强平价/安全距离不退出 → 单腿接近强平 → 后台退出双腿 → 原运行复盘
CROSSLINE_E2E_LIQUIDATION=1 node node_modules/@playwright/test/cli.js test --config test/e2e/paper-cycle.config.ts --grep "paper liquidation"
# 或：生成提醒正文/校验码 → 页面只读核验 → 篡改拒绝/原始时效到期 → 查看当前机会
node node_modules/@playwright/test/cli.js test --config test/e2e/paper-cycle.config.ts --grep "paper webhook"
# 或：链上执行详情 → 原记录收支复盘 → 失败重读/缺失记录 → 股票原币核算
node node_modules/@playwright/test/cli.js test --config test/e2e/paper-cycle.config.ts --grep "paper settlement review"
# 或：批量监控草稿 → 切页保存/失败恢复 → 暂停/过期与窄屏布局（无需编译 API）
CROSSLINE_E2E_PROFILE=release CROSSLINE_E2E_BROWSER_CHANNEL=chrome node node_modules/@playwright/test/cli.js test test/e2e/stocks-batch.spec.ts --workers=1
# 或：真实 BP 监控 worker → 批量询价 → 离页/返回 → 暂停取消旧请求 → 恢复
node node_modules/@playwright/test/cli.js test --config test/e2e/paper-cycle.config.ts --grep "paper BP batch"
# 或：单只股票缺卖价 → 整轮报价失败/退避 → 无需重新开启即可恢复
node node_modules/@playwright/test/cli.js test --config test/e2e/paper-cycle.config.ts --grep "paper BP failure"
# 或：单股修改参数 → 迟到报价 → 禁止旧数量预留 → 报价过期（隔离 HTTP/WS 夹具）
node node_modules/@playwright/test/cli.js test --config test/e2e/paper-cycle.config.ts --grep "BP quote edits"
```

该 E2E 启动实际 Rust HTTP/WS 路由、订单和复盘服务，使用固定候选与模拟账户，而不是伪造订单回包。
服务使用临时存储及 `18000/18080` 隔离端口，不加载 `.env`、私人账户或外部交易所适配器，不发送 Webhook，结束后自动停止。
`paper opportunity recovers` 只注入浏览器传输故障，订单、持仓和平仓仍由实际 Rust 模拟服务处理；断言始终仅一次开仓提交，恢复后的入口保持原运行范围。
历史入口的关键反例可独立运行：`CROSSLINE_E2E_PROFILE=release CROSSLINE_E2E_BROWSER_CHANNEL=chrome npx --no-install playwright test test/e2e/execution-orders.spec.ts --grep "history handoffs" --workers=1`。这条使用合成 HTTP/WS，检查无关成交、部分成交、全局历史、已收口和找不到指定运行时的导航，不提交订单。
`paper protection` 启动实际退出保护 worker，用测试行情触发止盈和止损，浏览器不发送手动平仓请求。
`paper liquidation` 使用显式开关启用合成强平价的隔离快照来源；持仓从实际模拟成交重建，距离使用原计算函数，退出、终态和复盘使用实际服务。仅在测试中替代账户快照发布器，不改变产品模拟账户，也不证明真实交易所强平价或账户流时效。
行情控制端点仅存在于带显式环境开关的隔离测试服务器，要求测试令牌，不进入产品路由。
`paper webhook` 使用真实票据服务与 Bark 正文编码器，不启动通知投递线程；它验证交接内容和浏览器核验，不证明手机送达。
`paper settlement review` 通过真实 Rust 收支接口与本地存储模型读取合成回执，验证链上、Backpack 和股票跨所显示，不代表真实账户或历史日志恢复验证。
`stocks-batch.spec.ts` 的两条路径拦截全部股票 HTTP/WS，以真实 WASM 和浏览器验证保存、暂停、报价过期、跨模块草稿/在途锁及拒绝后恢复；不证明真实 Provider 报价速度或订单成交。该独立页面验证不启动真实 Rust API。
`BP quote edits` 同样使用固定 HTTP/WS，验证当前参数与历史报价的区分及计划入口，不发送真实订单或资金预留。
`paper BP batch` 由浏览器操作真实 Rust 监控服务，只有外部 BP/RPC/Jupiter 使用本地替身；解析、配额、调度、批量缓存和应用 WS 均走原实现。它验证无查看者暂停、返回恢复、参数切换和旧请求取消，不读取真实密钥，不证明上游长期稳定性或真实市场报价速度。
`paper BP failure` 在同一隔离服务注入单只卖向和全部报价 HTTP 503，核验部分成功、连续失败退避、自动恢复、共享请求间隔、元数据缓存及单一 BP WS；没有访问真实报价源或发送订单。
前端独立声明 Rust workspace，嵌套交付副本不会误用外层工作区；需要复用已有缓存时可显式设置 `CARGO_TARGET_DIR`。
这些模拟场景不代表真实市场机会发现、实盘成交、真实账户强平安全、长期稳定性、盈利或全产品验收。

真实小额 place/cancel/finality 和私有流样本依赖操作员凭证。Fixture 与 parser 测试证明协议处理，
不能替代真实账户授权或实盘终态。

### 仓库结构

```text
crates/
  exchange/          交易所 adapter、REST/WS、规格与私有读写
  arbitrage/         五类策略、成本和收益证明
  trading/           订单状态机与执行账本
  portfolio/         持仓配对、风险与退出原语
  automation/        自动机会选择、门禁、冷却和状态
  onchain-monitor/   链上 / CEX 只读比较
  webhook/           目标校验、签名、队列与投递
  realtime/          AppWS hub、节流和历史
  review/            已执行、错过机会与策略复盘
  api/               Axum 边缘层和 orchestration service
shared-types/        前后端 DTO 单一事实源
frontend/            Leptos / Wasm 工作台
docs/                产品合同、传输矩阵与验收手册
scripts/             启停、诊断、发布和仓库门禁
third_party/         固定版本上游补丁与许可证
```
