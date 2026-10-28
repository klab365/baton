use bevy_ecs::prelude::{Added, Query, ResMut, Resource, With};
use rig_harness::prelude::{
    AgentLoop, App, AppCliExt, Front, IntoScheduleConfigs, Settled, Turn, TurnOutcome, Update,
};

#[derive(Default)]
pub(crate) struct StepPlugin;

#[derive(Resource, Default)]
pub(crate) struct Completed(pub(crate) Option<TurnOutcome>);

impl rig_harness::prelude::Plugin for StepPlugin {
    fn build(&self, app: &mut App) {
        app.add_front(Front::new("baton").on_terminal().headless())
            .init_resource::<Completed>()
            .add_systems(Update, complete.after(AgentLoop));
    }
}

fn complete(
    turns: Query<&TurnOutcome, (With<Turn>, Added<Settled>)>,
    mut result: ResMut<Completed>,
) {
    for outcome in &turns {
        result.0 = Some(outcome.clone());
    }
}
