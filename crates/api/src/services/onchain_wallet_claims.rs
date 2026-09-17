//! Shared wallet exclusivity, rebuilt from the owners' existing durable journals.
use fs2::FileExt;
use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    path::Path,
    sync::Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Module {
    Execution,
    CrossChain,
    Recovery,
    Replenishment,
    Approval,
    Stocks,
    StockFunding,
    StockStablecoin,
    StockExchangeConversion,
    StockPeer,
}

impl Module {
    fn label(self) -> &'static str {
        match self {
            Self::Execution => "链上 / CEX 执行",
            Self::CrossChain => "跨链执行",
            Self::Recovery => "资金处置计划",
            Self::Replenishment => "链上补库转账",
            Self::Approval => "代币授权",
            Self::Stocks => "股票套利计划",
            Self::StockFunding => "股票补库计划",
            Self::StockStablecoin => "股票稳定币兑换计划",
            Self::StockExchangeConversion => "Backpack 账户兑换计划",
            Self::StockPeer => "股票跨场所双边计划",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct WalletScope {
    chain: String,
    wallet: String,
}

impl WalletScope {
    pub(crate) fn new(chain: &str, wallet: &str) -> Result<Self, String> {
        let preset = shared_types::onchain_chain_preset(chain).ok_or("钱包占用记录的链未识别")?;
        let wallet = wallet.trim();
        if wallet.is_empty() {
            return Err("钱包占用记录缺少来源地址".into());
        }
        Ok(Self {
            chain: preset.id.into(),
            wallet: if preset.chain_id.is_some() {
                wallet.to_ascii_lowercase()
            } else {
                wallet.into()
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Owner {
    module: Module,
    id: String,
}

impl Owner {
    pub(crate) fn new(module: Module, id: &str) -> Self {
        Self {
            module,
            id: id.into(),
        }
    }
    fn busy(&self) -> String {
        format!(
            "钱包已由{} {} 占用；请先核验原交易或取消尚未提交的预留",
            self.module.label(),
            self.id
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hold {
    pub(crate) wallets: BTreeSet<WalletScope>,
    pub(crate) expires_at_ms: Option<i64>,
}

impl Hold {
    pub(crate) fn with_account(mut self, venue: &str, account_scope: &str) -> Result<Self, String> {
        if venue.is_empty() || account_scope.is_empty() {
            return Err("资金占用缺少交易所或账户范围".into());
        }
        self.wallets.insert(WalletScope {
            chain: format!("account:{}", venue.to_ascii_lowercase()),
            wallet: account_scope.into(),
        });
        Ok(self)
    }

    pub(crate) fn wallet(
        chain: &str,
        wallet: &str,
        expires_at_ms: Option<i64>,
    ) -> Result<Self, String> {
        Ok(Self {
            wallets: [WalletScope::new(chain, wallet)?].into_iter().collect(),
            expires_at_ms,
        })
    }
    fn active(&self, now: i64) -> bool {
        self.expires_at_ms.is_none_or(|t| now < t)
    }
}

#[derive(Debug, Default)]
struct Inner {
    holds: BTreeMap<Owner, Hold>,
    sources: BTreeSet<Module>,
    problems: BTreeMap<Module, String>,
}

#[derive(Debug, Default)]
pub(crate) struct WalletClaims {
    inner: Mutex<Inner>,
    _process_lock: Option<File>,
}

impl WalletClaims {
    pub(crate) fn exclusive(path: &Path) -> Result<Arc<Self>, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("钱包占用目录不可用：{e}"))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("钱包占用锁不可用：{e}"))?;
        file.try_lock_exclusive().map_err(|_| {
            "同一运行目录已有后端持有钱包占用锁，不能启动第二个资金执行实例".to_owned()
        })?;
        Ok(Arc::new(Self {
            inner: Mutex::default(),
            _process_lock: Some(file),
        }))
    }

    pub(crate) fn restore(
        &self,
        module: Module,
        claims: Result<Vec<(Owner, Hold)>, String>,
        problem: Option<String>,
    ) {
        let mut inner = self.inner.lock();
        if !inner.sources.insert(module) {
            inner
                .problems
                .insert(module, "同一模块重复装载资金日志".into());
            return;
        }
        match claims {
            Ok(claims) => {
                for (owner, hold) in claims {
                    inner.holds.insert(owner, hold);
                }
            }
            Err(e) => {
                inner.problems.insert(module, e);
            }
        }
        if let Some(problem) = problem {
            inner.problems.insert(module, problem);
        }
    }

    pub(crate) fn check(&self, chain: &str, wallet: &str, now: i64) -> Result<(), String> {
        let scope = WalletScope::new(chain, wallet)?;
        let inner = self.inner.lock();
        Self::healthy(&inner)?;
        if let Some((owner, _)) = inner
            .holds
            .iter()
            .find(|(_, h)| h.active(now) && h.wallets.contains(&scope))
        {
            return Err(owner.busy());
        }
        Ok(())
    }

    pub(crate) fn commit(
        &self,
        owner: Owner,
        hold: Option<Hold>,
        now: i64,
        persist: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock();
        if let Some(next) = hold.as_ref().filter(|h| h.active(now)) {
            if next.wallets.is_empty() {
                return Err("资金记录缺少可核验的钱包范围".into());
            }
            let prior = inner.holds.get(&owner).filter(|h| h.active(now));
            let added = next
                .wallets
                .iter()
                .filter(|w| prior.is_none_or(|p| !p.wallets.contains(*w)))
                .collect::<BTreeSet<_>>();
            if !added.is_empty() {
                Self::healthy(&inner)?;
                if let Some((other, _)) = inner.holds.iter().find(|(key, h)| {
                    **key != owner && h.active(now) && h.wallets.iter().any(|w| added.contains(w))
                }) {
                    return Err(other.busy());
                }
            }
        }
        // Check and fsync are serialized together. Restart replays this same owner journal.
        if let Err(problem) = persist() {
            inner.problems.insert(owner.module, problem.clone());
            return Err(problem);
        }
        if let Some(hold) = hold {
            inner.holds.insert(owner, hold);
        } else {
            inner.holds.remove(&owner);
        }
        Ok(())
    }

    pub(crate) fn persist_unclaimed(
        &self,
        module: Module,
        persist: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock();
        let result = persist();
        if let Err(e) = &result {
            inner.problems.insert(module, e.clone());
        }
        result
    }

    pub(crate) fn persist_unresolved(
        &self,
        module: Module,
        problem: String,
        persist: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock();
        inner.problems.insert(module, problem);
        // Legacy identity gaps block new spends, not observations of the original transfer.
        let result = persist();
        if let Err(e) = &result {
            inner.problems.insert(module, e.clone());
        }
        result
    }

    fn healthy(inner: &Inner) -> Result<(), String> {
        if let Some((module, _)) = inner.problems.iter().next() {
            return Err(format!(
                "{}资金日志未核清，不能证明共享钱包空闲；已阻止新增占用，请保留原日志核验",
                module.label()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
