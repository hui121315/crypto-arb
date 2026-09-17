//! Minimal i18n helpers kept for the current product shell.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub fn toggle(self) -> Self {
        match self {
            Lang::Zh => Lang::En,
            Lang::En => Lang::Zh,
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Lang::Zh => "中",
            Lang::En => "EN",
        }
    }
}

pub fn use_lang() -> RwSignal<Lang> {
    expect_context::<RwSignal<Lang>>()
}

/// Untranslated keys return themselves so missing product copy is visible.
pub fn tr(lang: Lang, key: &'static str) -> &'static str {
    match (lang, key) {
        (Lang::Zh, "funding.venues_label") => "交易所",
        (Lang::En, "funding.venues_label") => "Venues",
        _ => key,
    }
}
