use leptos::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckItemState {
    Ok,
    Warn,
    Block,
    Missing,
    Stale,
    Error,
    Unknown,
}

impl CheckItemState {
    pub(crate) fn class_name(self) -> &'static str {
        match self {
            Self::Ok => "check-item ok",
            Self::Warn => "check-item warn",
            Self::Block => "check-item block",
            Self::Missing => "check-item missing",
            Self::Stale => "check-item stale",
            Self::Error => "check-item error",
            Self::Unknown => "check-item unknown",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warn => "关注",
            Self::Block => "阻断",
            Self::Missing => "缺数据",
            Self::Stale => "过期",
            Self::Error => "错误",
            Self::Unknown => "未知",
        }
    }
}

#[component]
pub(in crate::panels) fn CheckItem<V, S>(
    #[prop(into)] label: String,
    value: V,
    state: S,
) -> impl IntoView
where
    V: IntoView + 'static,
    S: Fn() -> CheckItemState + Copy + Send + 'static,
{
    view! {
        <div class=move || state().class_name()>
            <span>{label}</span>
            <strong>{value}</strong>
            <em>{move || state().label()}</em>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::CheckItemState;

    #[test]
    fn check_item_state_maps_to_fail_closed_labels() {
        assert_eq!(CheckItemState::Ok.class_name(), "check-item ok");
        assert_eq!(CheckItemState::Ok.label(), "OK");
        assert_eq!(CheckItemState::Block.class_name(), "check-item block");
        assert_eq!(CheckItemState::Block.label(), "阻断");
        assert_eq!(CheckItemState::Missing.class_name(), "check-item missing");
        assert_eq!(CheckItemState::Missing.label(), "缺数据");
        assert_eq!(CheckItemState::Stale.label(), "过期");
        assert_eq!(CheckItemState::Error.label(), "错误");
        assert_eq!(CheckItemState::Unknown.label(), "未知");
    }
}
