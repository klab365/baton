use baton_core::domain::pipeline::ModelUsage;

/// Aggregates provider-reported turn counters without turning missing values into zero.
#[derive(Default, bevy_ecs::prelude::Resource)]
pub(crate) struct UsageAccumulator {
    input: Option<u64>,
    output: Option<u64>,
    cache_read: Option<u64>,
    cache_write: Option<u64>,
    overflowed: bool,
}

impl UsageAccumulator {
    pub(crate) fn record(&mut self, usage: &rig::completion::Usage) {
        self.overflowed |= add(&mut self.input, usage.input_tokens);
        self.overflowed |= add(&mut self.output, usage.output_tokens);
        self.overflowed |= add(&mut self.cache_read, usage.cached_input_tokens);
        self.overflowed |= add(&mut self.cache_write, usage.cache_creation_input_tokens);
    }

    pub(crate) fn finish(&self) -> Option<ModelUsage> {
        if self.overflowed
            || (self.input.is_none()
                && self.output.is_none()
                && self.cache_read.is_none()
                && self.cache_write.is_none())
        {
            return None;
        }
        Some(ModelUsage {
            input_tokens: self.input,
            output_tokens: self.output,
            cache_read_tokens: self.cache_read,
            cache_write_tokens: self.cache_write,
            estimated_cost: None,
            price_source: None,
            price_checked_at: None,
            currency: None,
            price_unit: None,
            coverage: Some("partial".into()),
            price_snapshot: None,
            input_price: None,
            output_price: None,
            cache_read_price: None,
            cache_write_price: None,
        })
    }
}

fn add(total: &mut Option<u64>, value: Option<u64>) -> bool {
    if let Some(value) = value {
        if let Some(sum) = total.unwrap_or_default().checked_add(value) {
            *total = Some(sum);
            false
        } else {
            true
        }
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::Usage;

    #[test]
    fn accumulates_reported_turns_and_keeps_unreported_categories_unknown() {
        let mut usage = UsageAccumulator::default();
        usage.record(
            &Usage::new()
                .input_tokens(10)
                .output_tokens(2)
                .cached_input_tokens(3),
        );
        usage.record(
            &Usage::new()
                .input_tokens(7)
                .output_tokens(1)
                .cache_creation_input_tokens(2),
        );
        let usage = usage.finish().unwrap();
        assert_eq!(usage.input_tokens, Some(17));
        assert_eq!(usage.output_tokens, Some(3));
        assert_eq!(usage.cache_read_tokens, Some(3));
        assert_eq!(usage.cache_write_tokens, Some(2));
    }

    #[test]
    fn overflow_makes_aggregate_unavailable_instead_of_wrapping() {
        let mut usage = UsageAccumulator::default();
        usage.record(&Usage::new().input_tokens(u64::MAX));
        usage.record(&Usage::new().input_tokens(1));
        assert_eq!(usage.finish(), None);
    }

    #[test]
    fn no_reported_counters_is_unavailable_not_zero() {
        let mut usage = UsageAccumulator::default();
        usage.record(&Usage::new());
        assert_eq!(usage.finish(), None);
    }
}
