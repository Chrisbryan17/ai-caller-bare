use polymarket_copybot::{JournalRecord, WalletLifecycle};

#[test]
fn rotation_journal_records_are_structured_and_secret_free() {
    let encoded = serde_json::to_string(&JournalRecord::WalletLifecycleChanged {
        observed_epoch: 100,
        wallet: "0x1111111111111111111111111111111111111111".into(),
        from: WalletLifecycle::Discovered,
        to: WalletLifecycle::Quarantined,
        reason: "historical_replay_passed".into(),
        active_set_generation: 1,
    })
    .unwrap();
    assert!(encoded.contains("wallet_lifecycle_changed"));
    assert!(encoded.contains("historical_replay_passed"));
    assert!(!encoded.to_ascii_lowercase().contains("private_key"));
}
