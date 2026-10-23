use baton_core::project::ProjectConfig;
use ts_rs::TS;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    ProjectConfig::export_all_to("web/lib/generated")?;
    Ok(())
}
