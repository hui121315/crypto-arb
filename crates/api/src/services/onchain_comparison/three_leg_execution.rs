use shared_types::{OnchainComparisonDirection, OnchainQuoteConversionSequence};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    QuoteConversion,
    PrimaryCex,
    Chain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecoveryAction {
    RetryQuoteConversion,
    ReversePrimaryCex,
    ReverseQuoteConversion,
    HoldHedgeAndAwaitChainFinality,
    FlagQuoteExposure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepOutcome {
    PartialFill,
    FinalityUnknown,
    Rejected,
}

pub(super) const MAX_QUOTE_CONVERSION_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Program {
    pub(super) steps: [Step; 3],
}

pub(super) fn program(
    direction: OnchainComparisonDirection,
    sequence: OnchainQuoteConversionSequence,
) -> Result<Program, String> {
    let expected = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => {
            OnchainQuoteConversionSequence::AfterPrimaryCex
        }
        OnchainComparisonDirection::BuyCexSellOnchain => {
            OnchainQuoteConversionSequence::BeforePrimaryCex
        }
    };
    if sequence != expected {
        return Err("Quote 换汇顺序与套利方向不一致".to_owned());
    }
    let steps = match sequence {
        OnchainQuoteConversionSequence::BeforePrimaryCex => {
            [Step::QuoteConversion, Step::PrimaryCex, Step::Chain]
        }
        OnchainQuoteConversionSequence::AfterPrimaryCex => {
            [Step::PrimaryCex, Step::Chain, Step::QuoteConversion]
        }
    };
    Ok(Program { steps })
}

pub(super) fn recovery_actions(
    program: Program,
    failed_step: Step,
    primary_filled: bool,
    conversion_filled: bool,
    chain_finality_unknown: bool,
) -> Vec<RecoveryAction> {
    if chain_finality_unknown {
        return vec![RecoveryAction::HoldHedgeAndAwaitChainFinality];
    }
    if failed_step == Step::QuoteConversion
        && program.steps[2] == Step::QuoteConversion
        && primary_filled
    {
        return vec![RecoveryAction::FlagQuoteExposure];
    }
    let mut actions = Vec::with_capacity(2);
    if primary_filled {
        actions.push(RecoveryAction::ReversePrimaryCex);
    }
    if conversion_filled {
        actions.push(RecoveryAction::ReverseQuoteConversion);
    }
    actions
}

pub(super) fn next_actions(
    program: Program,
    step: Step,
    outcome: StepOutcome,
    conversion_attempt: u8,
    primary_filled: bool,
    conversion_filled: bool,
) -> Vec<RecoveryAction> {
    if step == Step::Chain && outcome == StepOutcome::FinalityUnknown {
        return vec![RecoveryAction::HoldHedgeAndAwaitChainFinality];
    }
    if step == Step::QuoteConversion
        && outcome == StepOutcome::PartialFill
        && conversion_attempt < MAX_QUOTE_CONVERSION_ATTEMPTS
    {
        return vec![RecoveryAction::RetryQuoteConversion];
    }
    recovery_actions(program, step, primary_filled, conversion_filled, false)
}

pub(super) fn summary(program: Program) -> &'static str {
    match program.steps {
        [Step::QuoteConversion, Step::PrimaryCex, Step::Chain]
            if next_actions(program, Step::Chain, StepOutcome::Rejected, 1, true, true)
                == [
                    RecoveryAction::ReversePrimaryCex,
                    RecoveryAction::ReverseQuoteConversion,
                ] =>
        {
            "1 先换 Quote → 2 CEX 主单 → 3 链上成交；失败时按已成交腿逆序回滚"
        }
        [Step::PrimaryCex, Step::Chain, Step::QuoteConversion]
            if next_actions(
                program,
                Step::Chain,
                StepOutcome::FinalityUnknown,
                1,
                true,
                false,
            ) == [RecoveryAction::HoldHedgeAndAwaitChainFinality]
                && next_actions(
                    program,
                    Step::QuoteConversion,
                    StepOutcome::PartialFill,
                    MAX_QUOTE_CONVERSION_ATTEMPTS,
                    true,
                    false,
                ) == [RecoveryAction::FlagQuoteExposure] =>
        {
            "1 CEX 主单 → 2 链上成交 → 3 再换 Quote；链上未知时保留对冲，换汇失败标记 Quote 暴露"
        }
        _ => "三腿顺序无效",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_selects_the_only_safe_sequence() {
        let buy_chain = program(
            OnchainComparisonDirection::BuyOnchainSellCex,
            OnchainQuoteConversionSequence::AfterPrimaryCex,
        )
        .expect("buy-chain direction");
        assert_eq!(
            buy_chain.steps,
            [Step::PrimaryCex, Step::Chain, Step::QuoteConversion]
        );
        let buy_cex = program(
            OnchainComparisonDirection::BuyCexSellOnchain,
            OnchainQuoteConversionSequence::BeforePrimaryCex,
        )
        .expect("buy-cex direction");
        assert_eq!(
            buy_cex.steps,
            [Step::QuoteConversion, Step::PrimaryCex, Step::Chain]
        );
    }

    #[test]
    fn invalid_direction_sequence_is_rejected() {
        assert!(program(
            OnchainComparisonDirection::BuyOnchainSellCex,
            OnchainQuoteConversionSequence::BeforePrimaryCex,
        )
        .is_err());
    }

    #[test]
    fn pre_conversion_flow_rolls_back_filled_cex_legs_in_reverse_risk_order() {
        let program = program(
            OnchainComparisonDirection::BuyCexSellOnchain,
            OnchainQuoteConversionSequence::BeforePrimaryCex,
        )
        .expect("program");
        assert_eq!(
            recovery_actions(program, Step::Chain, true, true, false),
            vec![
                RecoveryAction::ReversePrimaryCex,
                RecoveryAction::ReverseQuoteConversion
            ]
        );
    }

    #[test]
    fn unknown_chain_finality_never_triggers_a_blind_reverse_order() {
        let program = program(
            OnchainComparisonDirection::BuyOnchainSellCex,
            OnchainQuoteConversionSequence::AfterPrimaryCex,
        )
        .expect("program");
        assert_eq!(
            recovery_actions(program, Step::Chain, true, false, true),
            vec![RecoveryAction::HoldHedgeAndAwaitChainFinality]
        );
    }

    #[test]
    fn post_conversion_failure_flags_quote_risk_without_unwinding_a_completed_hedge() {
        let program = program(
            OnchainComparisonDirection::BuyOnchainSellCex,
            OnchainQuoteConversionSequence::AfterPrimaryCex,
        )
        .expect("program");
        assert_eq!(
            recovery_actions(program, Step::QuoteConversion, true, false, false),
            vec![RecoveryAction::FlagQuoteExposure]
        );
    }

    #[test]
    fn partial_quote_conversion_retries_are_bounded() {
        let program = program(
            OnchainComparisonDirection::BuyCexSellOnchain,
            OnchainQuoteConversionSequence::BeforePrimaryCex,
        )
        .expect("program");
        assert_eq!(
            next_actions(
                program,
                Step::QuoteConversion,
                StepOutcome::PartialFill,
                1,
                false,
                true,
            ),
            vec![RecoveryAction::RetryQuoteConversion]
        );
        assert_eq!(
            next_actions(
                program,
                Step::QuoteConversion,
                StepOutcome::PartialFill,
                MAX_QUOTE_CONVERSION_ATTEMPTS,
                false,
                true,
            ),
            vec![RecoveryAction::ReverseQuoteConversion]
        );
    }

    #[test]
    fn exhausted_post_conversion_retry_only_flags_quote_exposure() {
        let program = program(
            OnchainComparisonDirection::BuyOnchainSellCex,
            OnchainQuoteConversionSequence::AfterPrimaryCex,
        )
        .expect("program");
        assert_eq!(
            next_actions(
                program,
                Step::QuoteConversion,
                StepOutcome::PartialFill,
                MAX_QUOTE_CONVERSION_ATTEMPTS,
                true,
                true,
            ),
            vec![RecoveryAction::FlagQuoteExposure]
        );
    }

    #[test]
    fn rejected_chain_unwinds_every_filled_cex_leg_in_safe_order() {
        let program = program(
            OnchainComparisonDirection::BuyCexSellOnchain,
            OnchainQuoteConversionSequence::BeforePrimaryCex,
        )
        .expect("program");
        assert_eq!(
            next_actions(program, Step::Chain, StepOutcome::Rejected, 1, true, true,),
            vec![
                RecoveryAction::ReversePrimaryCex,
                RecoveryAction::ReverseQuoteConversion
            ]
        );
    }
}
