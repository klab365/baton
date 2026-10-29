use baton_server::api::dto::ProjectConfig;
use ts_rs::TS;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    ProjectConfig::export_all_to("web/lib/generated")?;
    baton_server::api::dto::Skill::export_all_to("web/lib/generated")?;
    Ok(())
}
