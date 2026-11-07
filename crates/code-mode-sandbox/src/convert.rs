use rhai::{Array, Dynamic, Map};
use serde_json::{Map as JsonMap, Number, Value};

pub fn json_to_dynamic(value: &Value) -> Dynamic {
    match value {
        Value::Null => Dynamic::UNIT,
        Value::Bool(flag) => Dynamic::from(*flag),
        Value::Number(number) => number_to_dynamic(number),
        Value::String(text) => Dynamic::from(text.clone()),
        Value::Array(items) => {
            let array: Array = items.iter().map(json_to_dynamic).collect();
            Dynamic::from_array(array)
        }
        Value::Object(object) => Dynamic::from_map(json_object_to_map(object)),
    }
}

fn json_object_to_map(object: &JsonMap<String, Value>) -> Map {
    let mut map = Map::new();
    for (key, value) in object {
        map.insert(key.into(), json_to_dynamic(value));
    }
    map
}

fn number_to_dynamic(number: &Number) -> Dynamic {
    if let Some(value) = number.as_i64() {
        return Dynamic::from(value);
    }
    if let Some(value) = number.as_u64() {
        if value <= i64::MAX as u64 {
            return Dynamic::from(value as i64);
        }
        return Dynamic::from(number.to_string());
    }
    Dynamic::from(number.as_f64().unwrap_or(0.0))
}

pub fn dynamic_to_json(value: &Dynamic) -> Result<Value, String> {
    if value.is_unit() {
        return Ok(Value::Null);
    }
    if value.is_bool() {
        return Ok(Value::Bool(value.as_bool().unwrap_or(false)));
    }
    if value.is_int() {
        return value
            .as_int()
            .map(|int| Value::Number(int.into()))
            .map_err(|error| error.to_string());
    }
    if value.is_float() {
        let float = value.as_float().map_err(|error| error.to_string())?;
        return Number::from_f64(float)
            .map(Value::Number)
            .ok_or_else(|| format!("non-finite number {float}"));
    }
    if value.is_char() {
        return value
            .clone()
            .try_cast::<char>()
            .map(|ch| Value::String(ch.to_string()))
            .ok_or_else(|| "invalid char".into());
    }
    if value.is_string() {
        return Ok(Value::String(
            value
                .clone()
                .into_string()
                .map_err(|error| error.to_string())?,
        ));
    }
    if value.is_array() {
        let array = value
            .clone()
            .try_cast::<Array>()
            .ok_or_else(|| "invalid array".to_string())?;
        let mut items = Vec::with_capacity(array.len());
        for item in array {
            items.push(dynamic_to_json(&item)?);
        }
        return Ok(Value::Array(items));
    }
    if value.is_map() {
        let map = value
            .clone()
            .try_cast::<Map>()
            .ok_or_else(|| "invalid map".to_string())?;
        let mut object = JsonMap::new();
        for (key, item) in map {
            object.insert(key.to_string(), dynamic_to_json(&item)?);
        }
        return Ok(Value::Object(object));
    }
    Err(format!("unsupported script value `{}`", value.type_name()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trip_json() {
        let value = json!({
            "id": "page",
            "count": 2,
            "ok": true,
            "missing": null,
            "tags": ["a", "b"]
        });
        let dynamic = json_to_dynamic(&value);
        let back = dynamic_to_json(&dynamic).unwrap();
        assert_eq!(back, value);
    }
}
