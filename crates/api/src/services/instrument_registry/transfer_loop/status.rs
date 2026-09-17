#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CandidateTransferStatus {
    NotApplicable,
    NotRequired {
        detail: String,
    },
    Warming {
        detail: String,
    },
    Available {
        detail: String,
        base_network: String,
        quote_network: String,
        requires_tag: bool,
    },
    Blocked {
        detail: String,
    },
}

pub(super) fn from_blocker(reason: &str, prefix: &str) -> CandidateTransferStatus {
    let detail = reason.strip_prefix(prefix).unwrap_or(reason).to_owned();
    if reason.contains("正在按当前候选读取")
        || reason.contains("尚未取得官方充提网络状态")
        || reason.contains("官方充提网络状态已过期")
    {
        CandidateTransferStatus::Warming { detail }
    } else {
        CandidateTransferStatus::Blocked { detail }
    }
}
