use super::*;

impl VenueOperationKind {
    pub const fn label_zh(self) -> &'static str {
        match self {
            Self::PrivateRead => "私有读取",
            Self::OrderWrite => "写单运行态",
            Self::OrderFinality => "订单终态回查",
            Self::Balance => "余额缓存",
            Self::Positions => "持仓缓存",
            Self::OrderReconciliation => "订单回查",
            Self::CredentialProbeBalanceRead => "余额验证",
            Self::CredentialProbePositionsRead => "持仓验证",
            Self::CredentialProbeOpenOrdersRead => "挂单验证",
            Self::CredentialProbeAccountModeRead => "账户模式验证",
            Self::CredentialProbeOrderPermission => "交易权限验证",
            Self::PrivateWsSession => "私有 WS 会话",
            Self::PrivateWsSubscribe => "私有 WS 订阅",
            Self::PrivateWsOrderStream => "私有订单流",
            Self::PrivateWsAccountStream => "私有账户流",
            Self::AppWsBroadcast => "应用 WS 广播",
            Self::HttpRest => "HTTP 请求",
            Self::HostGate => "HostGate",
            Self::RateLimiter => "限速器",
            Self::RestOrderbooks => "REST 盘口",
            Self::RestFundingRates => "REST 资金费",
            Self::RestIndexCompositions => "REST 指数组成",
            Self::RestInstrumentSpecs => "REST 合约规格",
            Self::RestMetadata => "REST 合约元数据",
            Self::RestPerpTickers => "REST 永续报价",
            Self::RestSpotTicks => "REST 现货报价",
            Self::RestFundingFallback => "资金费 REST 回退",
            Self::RestTickerFallback => "行情 REST 回退",
            Self::WsFunding => "WS 资金费",
            Self::WsFundingSubscribe => "WS 资金费订阅",
            Self::WsFundingSnapshot => "WS 资金费快照",
            Self::WsTicker => "WS 行情",
            Self::WsTickerSubscribe => "WS 行情订阅",
            Self::WsTickerSnapshot => "WS 行情快照",
            Self::WsSpotSnapshot => "WS 现货行情快照",
            Self::OpportunitySnapshot => "机会原子快照",
            Self::WatchlistPrewarm => "自选公共预热",
            Self::BackgroundTasks => "后台任务总览",
            Self::BackgroundTask => "后台任务",
            Self::StorageAuditLog => "审计日志存储",
            Self::StorageHistory => "历史存储",
            Self::StoragePortfolioNav => "净值存储",
            Self::StorageExecutionLedger => "执行账本存储",
            Self::StorageOrderSnapshot => "订单快照存储",
            Self::StorageTradingSqlMigrations => "交易 SQL 迁移",
            Self::StorageTradingSqlLedger => "交易 SQL 账本写入",
            Self::StorageWatchlistAlerts => "自选提醒存储",
            Self::Unknown => "未知操作",
        }
    }

    pub const fn product_explanation_zh(self) -> &'static str {
        match self {
            Self::PrivateRead => {
                "表示当前 venue 的私有读取声明或运行态证据；字段/静态声明不等于可下单。"
            }
            Self::OrderWrite => {
                "表示实盘写单运行态是否有 live place/cancel/finality 证据；静态声明或 safe probe 不等于通过。"
            }
            Self::OrderFinality => {
                "表示未决 ExecutionRun/CloseRun 订单是否已由 REST order query 或私有 WS 证明终态。"
            }
            Self::Balance => "表示余额缓存的新鲜度、行数和错误状态。",
            Self::Positions => "表示持仓缓存的新鲜度、行数和错误状态。",
            Self::OrderReconciliation => "表示未决订单回查任务是否能证明 venue scoped 订单状态。",
            Self::CredentialProbeBalanceRead => "表示保存凭证后是否完成余额读取验证。",
            Self::CredentialProbePositionsRead => "表示保存凭证后是否完成持仓读取验证。",
            Self::CredentialProbeOpenOrdersRead => "表示保存凭证后是否完成未成交订单读取验证。",
            Self::CredentialProbeAccountModeRead => "表示保存凭证后是否完成账户模式读取验证。",
            Self::CredentialProbeOrderPermission => {
                "表示下单/撤单权限探针状态；Unknown 必须阻断实盘执行准入。"
            }
            Self::PrivateWsSession => "表示私有 WebSocket 连接会话是否有新鲜运行态样本。",
            Self::PrivateWsSubscribe => "表示私有 WebSocket 订阅请求是否构建并发送成功。",
            Self::PrivateWsOrderStream => {
                "表示私有订单事件流是否有新鲜订单样本，实盘未决订单会依赖它确认终态。"
            }
            Self::PrivateWsAccountStream => "表示私有账户事件流是否有新鲜账户或持仓样本。",
            Self::AppWsBroadcast => {
                "表示浏览器应用频道的 broadcast 接收器是否发生 lag，以及累计跳过了多少消息。"
            }
            Self::HttpRest => "表示交易所 HTTP endpoint 最近 outcome、延迟、限流和请求证据。",
            Self::HostGate => "表示同 host 的退避、熔断和 singleflight 压力状态。",
            Self::RateLimiter => "表示本地限速器最近等待、拒绝和父级限速状态。",
            Self::RestOrderbooks => "表示 REST 盘口读取在市场数据管线中的运行态。",
            Self::RestFundingRates => "表示 REST 资金费读取在市场数据管线中的运行态。",
            Self::RestIndexCompositions => "表示 REST 指数组成读取在市场数据管线中的运行态。",
            Self::RestInstrumentSpecs => "表示官方合约规格注册表的刷新、覆盖和新鲜度。",
            Self::RestMetadata => "表示合约、精度、乘数、费率间隔等市场元数据预热是否完成。",
            Self::RestPerpTickers => "表示 REST 永续报价读取在市场数据管线中的运行态。",
            Self::RestSpotTicks => "表示 REST 现货报价读取在市场数据管线中的运行态。",
            Self::RestFundingFallback => "表示 WS 资金费快照未就绪时 REST 回退是否成功。",
            Self::RestTickerFallback => "表示 WS 行情快照未就绪时 REST 回退是否成功。",
            Self::WsFunding => "表示 WebSocket 资金费推送在市场数据热路径中的运行态。",
            Self::WsFundingSubscribe => "表示资金费 WebSocket 订阅是否已触达并维持。",
            Self::WsFundingSnapshot => "表示资金费 WebSocket 是否已产出完整新鲜快照。",
            Self::WsTicker => "表示 WebSocket 行情推送在市场数据热路径中的运行态。",
            Self::WsTickerSubscribe => "表示行情 WebSocket 订阅是否已触达并维持。",
            Self::WsTickerSnapshot => "表示行情 WebSocket 是否已产出完整新鲜快照。",
            Self::WsSpotSnapshot => "表示现货 WebSocket 是否已产出完整新鲜快照。",
            Self::OpportunitySnapshot => "表示机会 id 索引是否以带版本的原子快照发布。",
            Self::WatchlistPrewarm => {
                "表示自选项触发的公共盘口/行情预热是否受限、去重并保留失败归因。"
            }
            Self::BackgroundTasks => "表示后端后台任务聚合健康状态。",
            Self::BackgroundTask => "表示单个后台任务是否 stale、down 或连续失败。",
            Self::StorageAuditLog => {
                "表示高风险操作审计 JSONL sink 是否已配置、可打开并记录写入失败。"
            }
            Self::StorageHistory => "表示历史行情和资金费存储后端是否可用。",
            Self::StoragePortfolioNav => "表示组合净值本地持久化是否可用。",
            Self::StorageExecutionLedger => "表示执行账本 JSONL append/replay 是否可用。",
            Self::StorageOrderSnapshot => "表示订单热投影快照 JSONL append/replay 是否可用。",
            Self::StorageTradingSqlMigrations => {
                "表示交易 SQL migration runner、schema version 与 checksum 是否可用。"
            }
            Self::StorageTradingSqlLedger => {
                "表示交易 SQL order_events/order_snapshots writer 是否可用。"
            }
            Self::StorageWatchlistAlerts => {
                "表示自选项、提醒规则和投递冷却状态的 SQLite 快照是否可恢复。"
            }
            Self::Unknown => "未被共享契约识别，必须从状态汇总和执行准入中排除。",
        }
    }
}
