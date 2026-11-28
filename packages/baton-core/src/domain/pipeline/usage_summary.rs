use super::StepRun;
use serde_json::{Value, json};

/// Totals estimates only when every persisted attempt has a complete comparable cost.
pub fn usage_summary(steps: &[StepRun]) -> Value {
    let unavailable =
        || json!({"cost":null,"currency":null,"priceUnit":null,"coverage":"unavailable"});
    let Some(first) = steps
        .first()
        .and_then(|step| step.usage.as_ref())
        .filter(|usage| usage.coverage.as_deref() == Some("complete"))
    else {
        return unavailable();
    };
    let (Some(currency), Some(unit)) = (&first.currency, &first.price_unit) else {
        return unavailable();
    };
    let values: Option<Vec<_>> = steps
        .iter()
        .map(|step| {
            let usage = step.usage.as_ref()?;
            if usage.coverage.as_deref() != Some("complete")
                || usage.currency.as_ref() != Some(currency)
                || usage.price_unit.as_ref() != Some(unit)
            {
                return None;
            }
            usage.estimated_cost.as_deref()
        })
        .collect();
    let Some(values) = values.filter(|_| !steps.is_empty()) else {
        return unavailable();
    };
    let parsed: Option<Vec<_>> = values
        .iter()
        .map(|value| {
            let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
            if whole.is_empty()
                || !whole.bytes().all(|b| b.is_ascii_digit())
                || !fraction.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            Some((whole, fraction))
        })
        .collect();
    let Some(parsed) = parsed else {
        return unavailable();
    };
    let scale = parsed
        .iter()
        .map(|(_, fraction)| fraction.len())
        .max()
        .unwrap_or(0);
    let Some(total) = parsed.iter().try_fold(0u128, |sum, (whole, fraction)| {
        sum.checked_add(format!("{whole}{fraction:0<scale$}").parse::<u128>().ok()?)
    }) else {
        return unavailable();
    };
    let digits = format!("{total:0>width$}", width = scale + 1);
    let cost = if scale == 0 {
        digits
    } else {
        let split = digits.len() - scale;
        let fraction = digits[split..].trim_end_matches('0');
        if fraction.is_empty() {
            digits[..split].to_string()
        } else {
            format!("{}.{}", &digits[..split], fraction)
        }
    };
    json!({"cost":cost,"currency":currency,"priceUnit":unit,"coverage":"complete"})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(cost: &str, currency: &str, unit: &str) -> StepRun {
        serde_json::from_value(json!({
            "stepId":"build", "agentId":"worker", "status":"succeeded",
            "input":null, "output":null, "error":null,
            "usage":{"estimatedCost":cost,"currency":currency,"priceUnit":unit,"coverage":"complete"}
        })).unwrap()
    }

    #[test]
    fn adds_exactly_and_rejects_incomparable_costs() {
        assert_eq!(
            usage_summary(&[
                step("0.1", "USD", "perMillionTokens"),
                step("0.02", "USD", "perMillionTokens"),
            ])["cost"],
            "0.12"
        );
        assert_eq!(
            usage_summary(&[
                step("0.1", "USD", "perMillionTokens"),
                step("0.02", "EUR", "perMillionTokens"),
            ])["coverage"],
            "unavailable"
        );
        assert_eq!(
            usage_summary(&[
                step("0.1", "USD", "perMillionTokens"),
                step("0.02", "USD", "perToken"),
            ])["coverage"],
            "unavailable"
        );
    }
}
