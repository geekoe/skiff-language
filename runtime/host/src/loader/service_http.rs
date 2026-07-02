use serde_json::{json, Value};

pub(super) fn service_http_hash_input(service: &Value) -> anyhow::Result<Option<Value>> {
    let Some(http) = service.get("http") else {
        return Ok(None);
    };
    let max_bytes = validate_service_http(http)?;
    parse_positive_usize(max_bytes)?;
    Ok(Some(json!({
        "response": {
            "maxBytes": max_bytes.clone(),
        },
    })))
}

pub(super) fn parse_service_http_response_max_bytes(
    assembly: &Value,
) -> anyhow::Result<Option<usize>> {
    let Some(service) = assembly.get("service") else {
        return Ok(None);
    };
    let Some(http) = service.get("http") else {
        return Ok(None);
    };
    let max_bytes = validate_service_http(http)?;
    parse_positive_usize(max_bytes).map(Some)
}

fn validate_service_http(http: &Value) -> anyhow::Result<&Value> {
    let object = http
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("service.http must be an object"))?;
    reject_unsupported_keys(object.keys(), &["response"], "service.http")?;

    let response = object.get("response").ok_or_else(|| {
        anyhow::anyhow!("service.http.response is required when service.http is present")
    })?;
    let response = response
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("service.http.response must be an object"))?;
    reject_unsupported_keys(response.keys(), &["maxBytes"], "service.http.response")?;
    response.get("maxBytes").ok_or_else(|| {
        anyhow::anyhow!("service.http.response.maxBytes is required when service.http is present")
    })
}

fn reject_unsupported_keys<'a>(
    keys: impl Iterator<Item = &'a String>,
    supported: &[&str],
    label: &str,
) -> anyhow::Result<()> {
    let unsupported = keys
        .filter(|key| !supported.contains(&key.as_str()))
        .map(|key| format!("{label}.{key}"))
        .collect::<Vec<_>>();
    if unsupported.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{label} does not support {}", unsupported.join(", "))
    }
}

fn parse_positive_usize(value: &Value) -> anyhow::Result<usize> {
    let number = if let Some(number) = value.as_u64() {
        Some(number)
    } else {
        let Some(number) = value.as_f64() else {
            return Err(anyhow::anyhow!(
                "service.http.response.maxBytes must be a positive integer"
            ));
        };

        if !number.is_finite() || number.fract() != 0.0 || number < 0.0 {
            return Err(anyhow::anyhow!(
                "service.http.response.maxBytes must be a positive integer"
            ));
        }

        if number > u64::MAX as f64 {
            return Err(anyhow::anyhow!(
                "service.http.response.maxBytes must fit within system integer size"
            ));
        }

        Some(number as u64)
    }
    .ok_or_else(|| anyhow::anyhow!("service.http.response.maxBytes must be a positive integer"))?;

    if number == 0 {
        anyhow::bail!("service.http.response.maxBytes must be greater than zero");
    }

    usize::try_from(number).map_err(|_| {
        anyhow::anyhow!("service.http.response.maxBytes must fit within system integer size")
    })
}
