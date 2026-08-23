use genegis_testkit::run_cloud_selected_view_benchmark;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let report = run_cloud_selected_view_benchmark()?;
    let failures = report.validate();
    if !failures.is_empty() {
        return Err(format!("selected-view evidence failed validation: {failures:?}").into());
    }
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(path, format!("{json}\n"))?;
    } else {
        println!("{json}");
    }
    Ok(())
}
