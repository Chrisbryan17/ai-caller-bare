use polymarket_copybot::{
    ActiveSetManager, CandidateEvaluation, CorrelationAnalyzer, Outcome, ReplayMetrics,
    RotationContext, SignalTrace, StrategyFamily, WalletRegistry,
};
use rust_decimal_macros::dec;
use tempfile::tempdir;

fn trace(condition: &str, outcome: Outcome, timestamp: i64) -> SignalTrace {
    SignalTrace {
        condition_id: condition.into(),
        outcome,
        timestamp,
    }
}

fn evaluation(wallet: &str, score: i64, traces: Vec<SignalTrace>) -> CandidateEvaluation {
    CandidateEvaluation {
        wallet: wallet.into(),
        family: StrategyFamily::FirstLargeBuy,
        estimated_win_probability: dec!(0.65),
        score: rust_decimal::Decimal::new(score, 0),
        eligible: true,
        rejection_reasons: vec![],
        metrics: ReplayMetrics {
            resolved_signals: 10,
            wins: 7,
            losses: 3,
            recent_twenty_four_hours: 5,
            recent_six_hours: 2,
            net_pnl_one_cent: dec!(2),
            net_pnl_two_cent: dec!(1),
            turnover_one_cent: dec!(5),
            roi_one_cent: dec!(0.4),
            max_loss_streak: 2,
            max_drawdown: dec!(1),
            median_price: dec!(0.3),
            median_lead_seconds: 120,
            two_cent_profitable_fraction: dec!(0.7),
            latest_signal_epoch: 10_000,
            signal_trace: traces,
        },
    }
}

#[test]
fn correlation_boundary_blocks_effective_duplicate_wallets() {
    let a: Vec<_> = (0..10)
        .map(|i| trace(&format!("c{i}"), Outcome::Up, i * 100))
        .collect();
    let b: Vec<_> = (0..8)
        .map(|i| trace(&format!("c{i}"), Outcome::Up, i * 100 + 3))
        .collect();
    let evidence = CorrelationAnalyzer::analyze(&a, &b);
    assert!(evidence.strongly_correlated);
    assert!(evidence.shared_condition_fraction >= dec!(0.70));
}

#[test]
fn rotation_preserves_incumbent_without_ten_percent_challenger_advantage() {
    let dir = tempdir().unwrap();
    let mut registry =
        WalletRegistry::load_or_new(dir.path().join("r.json"), dec!(95)).unwrap();
    for (wallet, score) in [("a", 100), ("b", 109)] {
        registry.upsert_evaluation(evaluation(wallet, score, vec![]), 0);
        registry.force_paper_qualified(wallet).unwrap();
    }
    let proposal = ActiveSetManager::default().propose(
        &registry,
        &["a".into()],
        RotationContext {
            position_open: false,
            submission_in_flight: false,
            state_persisted: true,
        },
    );
    assert_eq!(proposal.wallets, vec!["a"]);
}

#[test]
fn rotation_is_deferred_while_position_is_open() {
    let dir = tempdir().unwrap();
    let registry = WalletRegistry::load_or_new(dir.path().join("r.json"), dec!(95)).unwrap();
    let proposal = ActiveSetManager::default().propose(
        &registry,
        &["a".into()],
        RotationContext {
            position_open: true,
            submission_in_flight: false,
            state_persisted: true,
        },
    );
    assert!(proposal.deferred);
    assert_eq!(proposal.wallets, vec!["a"]);
}
