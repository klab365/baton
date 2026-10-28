use std::{env, fmt, path::PathBuf};

use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    fmt::{FmtContext, FormatEvent, FormatFields, format::Writer},
    registry::LookupSpan,
};

/// Suppress Rig's known, non-semantic ChatGPT stream heartbeat while leaving
/// its payload in Rig's response stream for its raw-session handling.
struct BatonEventFormat;

impl<S, N> FormatEvent<S, N> for BatonEventFormat
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        if is_rig_response_metadata(event) {
            return Ok(());
        }
        tracing_subscriber::fmt::format().format_event(ctx, writer, event)
    }
}

fn is_rig_response_metadata(event: &Event<'_>) -> bool {
    if event.metadata().target() != "rig_core::driver" {
        return false;
    }
    let mut visitor = EventKind::default();
    event.record(&mut visitor);
    visitor.0.as_deref() == Some("response.metadata")
}

#[derive(Default)]
struct EventKind(Option<String>);

impl Visit for EventKind {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "kind" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn fmt::Debug) {}
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .event_format(BatonEventFormat)
        .with_env_filter(filter)
        .init();
    let project_dir = env::var_os("BATON_PROJECT_DIR")
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?);
    let host = env::var("BATON_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = env::var("BATON_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5173);
    println!(
        "Baton API listening on http://{host}:{port} for {}",
        project_dir.display()
    );
    baton_server::serve(project_dir, &host, port).await
}
