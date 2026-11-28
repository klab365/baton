use serde_json::Value;
use std::{
    sync::{OnceLock, RwLock},
    time::{Duration, Instant},
};

const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const CACHE_TTL: Duration = Duration::from_secs(3600);
static CATALOG: OnceLock<RwLock<Option<(Instant, Value)>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq)]
pub struct ModelPrices {
    pub input: Option<String>,
    pub output: Option<String>,
    pub cache_read: Option<String>,
    pub cache_write: Option<String>,
}

fn parse_catalog_response(status: u16, value: Value) -> Option<Value> {
    ((200..300).contains(&status) && value.is_object()).then_some(value)
}

fn find_model_prices(catalog: &Value, provider: &str, model: &str) -> Option<ModelPrices> {
    let cost = catalog
        .get(provider)?
        .get("models")?
        .get(model)?
        .get("cost")?;
    let price = |key: &str| {
        let number = cost.get(key)?.as_number()?;
        (number.as_f64()? >= 0.0).then(|| number.to_string())
    };
    Some(ModelPrices {
        input: price("input"),
        output: price("output"),
        cache_read: price("cache_read"),
        cache_write: price("cache_write"),
    })
}

async fn fetch_catalog(url: &str) -> Option<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;
    let response = client.get(url).send().await.ok()?;
    let status = response.status().as_u16();
    let body = response.bytes().await.ok()?;
    let value = serde_json::from_slice::<Value>(&body).ok()?;
    parse_catalog_response(status, value)
}

async fn catalog() -> Option<Value> {
    if let Some(cached) = CATALOG
        .get_or_init(|| RwLock::new(None))
        .read()
        .ok()?
        .as_ref()
        .filter(|(at, _)| at.elapsed() < CACHE_TTL)
        .map(|(_, value)| value.clone())
    {
        return Some(cached);
    }
    let value = fetch_catalog(MODELS_DEV_URL).await?;
    if let Ok(mut cache) = CATALOG.get_or_init(|| RwLock::new(None)).write() {
        *cache = Some((Instant::now(), value.clone()));
    }
    Some(value)
}

/// Price lookup is best-effort and bounded; callers must treat `None` as unavailable.
pub async fn lookup(provider: &str, model: &str) -> Option<ModelPrices> {
    let catalog = catalog().await?;
    find_model_prices(&catalog, provider, model)
}

pub fn apply_prices(
    mut usage: baton_core::domain::pipeline::ModelUsage,
    prices: &ModelPrices,
    checked_at: String,
) -> baton_core::domain::pipeline::ModelUsage {
    usage.input_price = prices.input.clone();
    usage.output_price = prices.output.clone();
    usage.cache_read_price = prices.cache_read.clone();
    usage.cache_write_price = prices.cache_write.clone();
    usage.price_source = Some("models.dev".into());
    usage.price_checked_at = Some(checked_at);
    usage.currency = Some("USD".into());
    usage.price_unit = Some("perMillionTokens".into());
    usage.price_snapshot = Some(serde_json::json!({
        "input": prices.input, "output": prices.output,
        "cacheRead": prices.cache_read, "cacheWrite": prices.cache_write,
    }));
    usage.estimated_cost = estimate_cost(&usage, prices);
    usage.coverage = Some(
        if usage.estimated_cost.is_some() {
            "complete"
        } else {
            "partial"
        }
        .into(),
    );
    usage
}

/// Exact USD estimate for per-million-token rates. Unknown counters/rates or unsupported
/// rate notation produce `None`; cached tokens are billed in their own categories.
pub fn estimate_cost(
    usage: &baton_core::domain::pipeline::ModelUsage,
    prices: &ModelPrices,
) -> Option<String> {
    let input = usage.input_tokens?;
    let output = usage.output_tokens?;
    let cached_read = usage.cache_read_tokens;
    let cached_write = usage.cache_write_tokens;
    if (cached_read.is_some() && prices.cache_read.is_none())
        || (cached_write.is_some() && prices.cache_write.is_none())
        || (input > 0 && (cached_read.is_none() || cached_write.is_none()))
    {
        return None;
    }
    let cached_read = cached_read.unwrap_or(0);
    let cached_write = cached_write.unwrap_or(0);
    let cached = cached_read.checked_add(cached_write)?;
    let uncached_input = input.checked_sub(cached)?;
    let mut numerator = 0u128;
    for (tokens, rate) in [
        (uncached_input, prices.input.as_deref()),
        (output, prices.output.as_deref()),
        (cached_read, prices.cache_read.as_deref()),
        (cached_write, prices.cache_write.as_deref()),
    ] {
        if tokens == 0 {
            continue;
        }
        let rate = decimal_rate(rate?)?;
        numerator = numerator.checked_add(u128::from(tokens).checked_mul(rate)?)?;
    }
    let denominator = 1_000_000_000_000_000u128;
    let whole = numerator / denominator;
    let fraction = format!("{:015}", numerator % denominator);
    let fraction = fraction.trim_end_matches('0');
    Some(if fraction.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{fraction}")
    })
}

fn decimal_rate(rate: &str) -> Option<u128> {
    let (whole, fraction) = rate.split_once('.').unwrap_or((rate, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 9
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let scaled = format!("{whole}{fraction:0<9}");
    scaled.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetches_and_matches_provider_model_catalog() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request).await;
            let body = r#"{"openai":{"models":{"gpt-x":{"cost":{"input":2,"output":8}}}}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let catalog = fetch_catalog(&format!("http://{address}")).await.unwrap();
        assert_eq!(
            find_model_prices(&catalog, "openai", "gpt-x")
                .unwrap()
                .output
                .as_deref(),
            Some("8")
        );
    }

    #[tokio::test]
    async fn http_error_is_nonfatal() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request).await;
            socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}").await.unwrap();
        });
        assert_eq!(fetch_catalog(&format!("http://{address}")).await, None);
    }

    #[test]
    fn applies_cost_and_provenance_to_usage() {
        let usage = baton_core::domain::pipeline::ModelUsage {
            input_tokens: Some(1_000_000),
            output_tokens: Some(0),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
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
        };
        let prices = ModelPrices {
            input: Some("2".into()),
            output: Some("8".into()),
            cache_read: Some("0.2".into()),
            cache_write: Some("1".into()),
        };
        let result = apply_prices(usage, &prices, "now".into());
        assert_eq!(result.estimated_cost.as_deref(), Some("2"));
        assert_eq!(result.currency.as_deref(), Some("USD"));
        assert_eq!(result.price_source.as_deref(), Some("models.dev"));
        assert_eq!(result.price_checked_at.as_deref(), Some("now"));
        assert_eq!(result.input_price.as_deref(), Some("2"));
        assert_eq!(result.coverage.as_deref(), Some("complete"));
    }

    #[test]
    fn estimates_each_category_without_double_counting_cached_input() {
        let usage = baton_core::domain::pipeline::ModelUsage {
            input_tokens: Some(10),
            output_tokens: Some(4),
            cache_read_tokens: Some(2),
            cache_write_tokens: Some(3),
            estimated_cost: None,
            price_source: None,
            price_checked_at: None,
            currency: None,
            price_unit: None,
            coverage: None,
            price_snapshot: None,
            input_price: None,
            output_price: None,
            cache_read_price: None,
            cache_write_price: None,
        };
        let prices = ModelPrices {
            input: Some("2".into()),
            output: Some("8".into()),
            cache_read: Some("0.2".into()),
            cache_write: Some("1".into()),
        };
        assert_eq!(estimate_cost(&usage, &prices).as_deref(), Some("0.0000454"));
    }

    #[test]
    fn matches_exact_provider_and_model_and_preserves_missing_categories() {
        let catalog = serde_json::json!({"openai":{"models":{"gpt-x":{"cost":{"input":2.0,"output":8.0,"cache_read":0.2,"cache_write":1.0}}}}});
        assert_eq!(
            find_model_prices(&catalog, "openai", "gpt-x"),
            Some(ModelPrices {
                input: Some("2.0".into()),
                output: Some("8.0".into()),
                cache_read: Some("0.2".into()),
                cache_write: Some("1.0".into()),
            })
        );
        assert_eq!(find_model_prices(&catalog, "other", "gpt-x"), None);
        assert_eq!(find_model_prices(&catalog, "openai", "unknown"), None);
    }

    #[test]
    fn rejects_error_status_and_non_object_responses() {
        assert_eq!(
            parse_catalog_response(503, serde_json::json!({"x":{}})),
            None
        );
        assert_eq!(parse_catalog_response(200, serde_json::json!([])), None);
        assert_eq!(
            parse_catalog_response(200, serde_json::json!({"x":{}})),
            Some(serde_json::json!({"x":{}}))
        );
    }

    #[test]
    fn preserves_decimal_rate_precision_as_text() {
        let catalog: Value = serde_json::from_str(
            r#"{"p":{"models":{"m":{"cost":{"input":0.000000000123456789}}}}}"#,
        )
        .unwrap();
        assert_eq!(
            find_model_prices(&catalog, "p", "m")
                .unwrap()
                .input
                .as_deref(),
            Some("1.23456789e-10")
        );
    }

    #[test]
    fn missing_or_string_category_rates_remain_unknown() {
        let catalog = serde_json::json!({"openai":{"models":{"x":{"cost":{"input":"2","output":0,"cache_read":-1}}}}});
        let prices = find_model_prices(&catalog, "openai", "x").unwrap();
        assert_eq!(prices.input, None);
        assert_eq!(prices.output, Some("0".into()));
        assert_eq!(prices.cache_read, None);
        assert_eq!(prices.cache_write, None);
    }

    #[test]
    fn malformed_catalog_or_cost_is_unavailable_not_zero() {
        assert_eq!(
            find_model_prices(&serde_json::json!({}), "openai", "x"),
            None
        );
        let catalog = serde_json::json!({"openai":{"models":{"x":{"cost":{"input":"2"}}}}});
        let prices = find_model_prices(&catalog, "openai", "x").unwrap();
        assert_eq!(prices.input, None);
        assert_eq!(prices.output, None);
    }
}
