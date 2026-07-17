use polymarket_copybot::{
    ExecutionFill, JournalRecord, JsonlJournal, Outcome,
};
use rust_decimal_macros::dec;

#[tokio::test]
async fn journal_writes_one_valid_redacted_json_record_per_line() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("copybot.jsonl");
    let journal = JsonlJournal::open(&path).await.unwrap();
    journal
        .append(&JournalRecord::Fill {
            observed_epoch: 123,
            fill: ExecutionFill {
                condition_id: "condition".into(),
                asset_id: "asset".into(),
                outcome: Outcome::Up,
                shares: dec!(5),
                fill_price: dec!(0.41),
                fee: dec!(0.084665),
                total_cost: dec!(2.134665),
                external_id: None,
                paper: true,
            },
        })
        .await
        .unwrap();

    let raw = tokio::fs::read_to_string(path).await.unwrap();
    assert_eq!(raw.lines().count(), 1);
    let parsed: serde_json::Value = serde_json::from_str(raw.trim()).unwrap();
    assert_eq!(parsed["kind"], "fill");
    assert_eq!(parsed["observed_epoch"], 123);
    assert!(!raw.to_ascii_lowercase().contains("private_key"));
    assert!(!raw.to_ascii_lowercase().contains("secret"));
}
