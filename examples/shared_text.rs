//! Two agents editing one document. Run with `cargo run --example shared_text --features text`.

fn main() -> Result<(), llm_sync::SyncError> {
use llm_sync::SharedText;


    let mut planner = SharedText::new(1);           // one id per agent
    planner.push("1. research\n");
    let mut writer = SharedText::from_update(2, &planner.encode_state())?;

    planner.push("2. outline\n");                    // both edit at once
    writer.insert(0, "PLAN\n")?;

    // Send each side only what the other is missing.
    writer.apply_update(&planner.encode_diff(&writer.state_vector())?)?;
    planner.apply_update(&writer.encode_diff(&planner.state_vector())?)?;

    assert_eq!(planner.text(), "PLAN\n1. research\n2. outline\n");
    assert_eq!(planner.text(), writer.text());
    println!("{}", planner.text());
    Ok(())
}
