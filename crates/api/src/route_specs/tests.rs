use super::*;

#[test]
fn cross_chain_recheck_and_submit_have_separate_authenticated_contracts() {
    let rows = seeded_endpoint_registry_summary(&ApiSurfaceConfig::default());
    for (path, risk) in [
        ("/api/onchain/cross-chain/recheck", "low"),
        ("/api/onchain/cross-chain/submit", "high"),
        ("/api/onchain/replenishment/recheck", "low"),
        ("/api/onchain/replenishment/submit", "high"),
    ] {
        let row = rows
            .iter()
            .find(|row| row.1 == path)
            .expect("registered route");
        assert_eq!(row.0, "POST");
        assert_eq!(row.4, risk);
        assert_eq!(row.5, "bearer");
        assert_eq!(row.6, "required");
    }
}

#[test]
fn disabled_route_summary_is_unique_sorted_and_stable() {
    let surface = ApiSurfaceConfig::default();
    let first = disabled_route_registry_summary(&surface);
    let second = disabled_route_registry_summary(&surface);

    assert_eq!(first, second);
    assert!(first.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        first
            .iter()
            .filter(|flag| **flag == "api_surface.watchlist_alerts")
            .count(),
        1,
        "watchlist and alerts share one runtime feature flag"
    );
}

#[test]
fn enabling_compiled_shared_gate_removes_disabled_feature() {
    let mut surface = ApiSurfaceConfig {
        watchlist_alerts: true,
        ..ApiSurfaceConfig::default()
    };

    assert!(!disabled_route_registry_summary(&surface).contains(&"api_surface.watchlist_alerts"));

    surface.watchlist_alerts = false;
    assert!(disabled_route_registry_summary(&surface).contains(&"api_surface.watchlist_alerts"));
}

#[cfg(not(feature = "legacy-chat"))]
#[test]
fn requested_uncompiled_chat_surfaces_are_operator_visible() {
    let surface = ApiSurfaceConfig {
        chat: true,
        llm_diagnostics: true,
        ..ApiSurfaceConfig::default()
    };
    let disabled = disabled_route_registry_summary(&surface);

    assert!(disabled.contains(&CHAT_REQUESTED_BUT_UNCOMPILED));
    assert!(disabled.contains(&LLM_REQUESTED_BUT_UNCOMPILED));
}

#[cfg(not(feature = "legacy-options"))]
#[test]
fn requested_uncompiled_options_surface_is_operator_visible() {
    let surface = ApiSurfaceConfig {
        options: true,
        ..ApiSurfaceConfig::default()
    };

    assert!(disabled_route_registry_summary(&surface).contains(&OPTIONS_REQUESTED_BUT_UNCOMPILED));
}
