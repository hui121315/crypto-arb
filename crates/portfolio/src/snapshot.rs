use arc_swap::ArcSwap;
use shared_types::PortfolioSnapshot;
use std::sync::Arc;

pub type SharedPortfolioSnapshot = Arc<ArcSwap<PortfolioSnapshot>>;

pub fn new_shared(initial: PortfolioSnapshot) -> SharedPortfolioSnapshot {
    Arc::new(ArcSwap::from_pointee(initial))
}
