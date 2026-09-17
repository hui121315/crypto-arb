use shared_types::{ExecutionEnvironment, ExecutionMode};

/// Product-facing labels deliberately collapse legacy DryRun/Testnet transport modes.
pub(in crate::panels) const fn execution_environment_label(
    environment: ExecutionEnvironment,
) -> &'static str {
    match environment {
        ExecutionEnvironment::Paper => "模拟",
        ExecutionEnvironment::Live => "实盘",
    }
}

pub(in crate::panels) const fn execution_mode_label(mode: ExecutionMode) -> &'static str {
    execution_environment_label(mode.environment())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_mode_and_environment_labels_hide_compatibility_variants() {
        assert_eq!(execution_mode_label(ExecutionMode::DryRun), "模拟");
        assert_eq!(execution_mode_label(ExecutionMode::Testnet), "模拟");
        assert_eq!(execution_mode_label(ExecutionMode::Live), "实盘");
        assert_eq!(
            execution_environment_label(ExecutionEnvironment::Paper),
            "模拟"
        );
    }
}
