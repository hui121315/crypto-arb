#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BaselineFeed {
    PerpTickers,
    SpotTicks,
}

impl BaselineFeed {
    pub(super) const COUNT: usize = 2;

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::PerpTickers => "perp_tickers",
            Self::SpotTicks => "spot_ticks",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BaselineShard {
    pub(super) venue: String,
    pub(super) feed: BaselineFeed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BaselineRefresh {
    None,
    Shard(BaselineShard),
}

impl BaselineRefresh {
    pub(super) const fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Shard(_) => "shard",
        }
    }

    pub(super) fn venue(&self) -> Option<&str> {
        match self {
            Self::Shard(shard) => Some(&shard.venue),
            Self::None => None,
        }
    }

    pub(super) fn feed(&self) -> Option<&'static str> {
        match self {
            Self::Shard(shard) => Some(shard.feed.label()),
            Self::None => None,
        }
    }
}

pub(super) fn baseline_shard_at(venues: &[String], cursor: usize) -> Option<BaselineShard> {
    let slot_count = venues.len().checked_mul(BaselineFeed::COUNT)?;
    if slot_count == 0 {
        return None;
    }
    let slot = cursor % slot_count;
    let venue = venues.get(slot / BaselineFeed::COUNT)?.clone();
    let feed = if slot % BaselineFeed::COUNT == 0 {
        BaselineFeed::PerpTickers
    } else {
        BaselineFeed::SpotTicks
    };
    Some(BaselineShard { venue, feed })
}

pub(super) fn advance_baseline_cursor(cursor: usize, venue_count: usize) -> usize {
    let Some(slot_count) = venue_count.checked_mul(BaselineFeed::COUNT) else {
        return 0;
    };
    if slot_count == 0 {
        return 0;
    }
    cursor.wrapping_add(1) % slot_count
}
