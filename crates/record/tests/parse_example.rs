use record::Record;

const EXAMPLE: &str = include_str!("../../../evals/schema/examples/record-v0.json");

#[test]
fn parses_record_v0_example() {
    let record: Record = serde_json::from_str(EXAMPLE).expect("example should parse");

    assert_eq!(record.schema_version, "record-v0");
    assert_eq!(record.run.task_id, "read-echo-01");
    assert_eq!(record.events.len(), 1);
}
