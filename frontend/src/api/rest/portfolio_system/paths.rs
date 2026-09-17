use super::super::*;

pub(super) fn close_position_path(venue: &str, symbol: &str) -> String {
    let venue = encode_path_segment(venue);
    let symbol = encode_path_segment(symbol);
    format!("/api/trading/portfolio/positions/{venue}/{symbol}/close")
}

pub(super) fn close_position_pair_path(venue: &str, symbol: &str) -> String {
    let venue = encode_path_segment(venue);
    let symbol = encode_path_segment(symbol);
    format!("/api/trading/portfolio/positions/{venue}/{symbol}/close-pair")
}

pub(super) fn close_run_compensation_path(close_run_id: &str) -> String {
    let close_run_id = encode_path_segment(close_run_id);
    format!("/api/trading/portfolio/close-runs/{close_run_id}/compensation-orders")
}

pub(super) fn close_run_manual_terminal_path(close_run_id: &str) -> String {
    let close_run_id = encode_path_segment(close_run_id);
    format!("/api/trading/portfolio/close-runs/{close_run_id}/manual-terminal")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portfolio_position_paths_encode_each_dynamic_segment() {
        let venue = "venue%2Fa%3Fx";
        let symbol = "BTC%2FUSDT%20%23perp";

        assert_eq!(
            close_position_path("venue/a?x", "BTC/USDT #perp"),
            format!("/api/trading/portfolio/positions/{venue}/{symbol}/close")
        );
        assert_eq!(
            close_position_pair_path("venue/a?x", "BTC/USDT #perp"),
            format!("/api/trading/portfolio/positions/{venue}/{symbol}/close-pair")
        );
    }

    #[test]
    fn close_run_paths_encode_the_identifier_as_one_segment() {
        let id = "close/run?retry=1#leg %";
        let encoded = "close%2Frun%3Fretry%3D1%23leg%20%25";

        assert_eq!(
            close_run_compensation_path(id),
            format!("/api/trading/portfolio/close-runs/{encoded}/compensation-orders")
        );
        assert_eq!(
            close_run_manual_terminal_path(id),
            format!("/api/trading/portfolio/close-runs/{encoded}/manual-terminal")
        );
    }
}
