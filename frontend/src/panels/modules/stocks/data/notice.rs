use leptos::prelude::*;

#[derive(Clone)]
pub(in crate::panels::modules::stocks) struct NoticeMessage {
    pub message: String,
    pub problem: bool,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::stocks) struct Notice(RwSignal<Option<NoticeMessage>>);

impl Notice {
    pub(in crate::panels::modules::stocks) fn new() -> Self {
        Self(RwSignal::new(None))
    }

    pub(in crate::panels::modules::stocks) fn get(self) -> Option<NoticeMessage> {
        self.0.get()
    }

    pub(in crate::panels::modules::stocks) fn set(self, value: Option<String>) {
        self.0.set(value.map(|message| NoticeMessage { message, problem: true }));
    }

    pub(in crate::panels::modules::stocks) fn try_set(self, value: Option<String>) {
        self.0.try_set(value.map(|message| NoticeMessage { message, problem: true }));
    }

    pub(in crate::panels::modules::stocks) fn inform(self, message: impl Into<String>) {
        self.0.try_set(Some(NoticeMessage { message: message.into(), problem: false }));
    }
}
