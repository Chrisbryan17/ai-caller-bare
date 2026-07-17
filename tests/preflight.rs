use polymarket_copybot::{PreflightFacts, PreflightStatus};
use rust_decimal_macros::dec;

#[test]
fn preflight_requires_trading_access_balance_allowance_signer_and_no_open_orders() {
    let passed = PreflightStatus::from_facts(
        &PreflightFacts {
            geoblocked: false,
            closed_only: false,
            balance: dec!(95),
            has_positive_allowance: true,
            signer_authenticated: true,
            open_orders: 0,
        },
        dec!(4.75),
    );
    assert!(passed.passed());

    for facts in [
        PreflightFacts {
            geoblocked: true,
            ..passed_facts()
        },
        PreflightFacts {
            closed_only: true,
            ..passed_facts()
        },
        PreflightFacts {
            balance: dec!(4.74),
            ..passed_facts()
        },
        PreflightFacts {
            has_positive_allowance: false,
            ..passed_facts()
        },
        PreflightFacts {
            signer_authenticated: false,
            ..passed_facts()
        },
        PreflightFacts {
            open_orders: 1,
            ..passed_facts()
        },
    ] {
        assert!(!PreflightStatus::from_facts(&facts, dec!(4.75)).passed());
    }
}

fn passed_facts() -> PreflightFacts {
    PreflightFacts {
        geoblocked: false,
        closed_only: false,
        balance: dec!(95),
        has_positive_allowance: true,
        signer_authenticated: true,
        open_orders: 0,
    }
}
