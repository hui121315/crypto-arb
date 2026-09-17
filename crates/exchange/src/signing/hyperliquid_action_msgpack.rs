//! Protocol-ordered msgpack encoding for Hyperliquid L1 actions.

use serde::ser::{Error as _, SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use serde_json::{Map, Number, Value};

const ORDER_ACTION_KEYS: &[&str] = &["type", "orders", "grouping", "builder"];
const ORDER_ACTION_REQUIRED: &[&str] = &["type", "orders", "grouping"];
const CANCEL_ACTION_KEYS: &[&str] = &["type", "cancels"];
const ORDER_KEYS: &[&str] = &["a", "b", "p", "s", "r", "t", "c"];
const ORDER_REQUIRED: &[&str] = &["a", "b", "p", "s", "r", "t"];
const ORDER_TYPE_KEYS: &[&str] = &["limit", "trigger"];
const LIMIT_KEYS: &[&str] = &["tif"];
const TRIGGER_KEYS: &[&str] = &["isMarket", "triggerPx", "tpsl"];
const CANCEL_KEYS: &[&str] = &["a", "o"];
const CANCEL_BY_CLOID_KEYS: &[&str] = &["asset", "cloid"];
const BUILDER_KEYS: &[&str] = &["b", "f"];
const GROUPING_KEYS: &[&str] = &["p"];
const DUMMY_KEYS: &[&str] = &["type", "num"];
const TYPE_ONLY_KEYS: &[&str] = &["type"];

pub(super) fn to_vec(action: &Value) -> Result<Vec<u8>, rmp_serde::encode::Error> {
    rmp_serde::to_vec_named(&OfficialAction(action))
}

struct OfficialAction<'a>(&'a Value);

impl Serialize for OfficialAction<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        OfficialValue {
            value: self.0,
            shape: Shape::Action,
        }
        .serialize(serializer)
    }
}

#[derive(Clone, Copy)]
enum Shape {
    Action,
    Orders,
    Order,
    OrderType,
    Limit,
    Trigger,
    Cancels(CancelKind),
    Cancel,
    CancelByCloid,
    Builder,
    Grouping,
    Scalar,
}

#[derive(Clone, Copy)]
enum CancelKind {
    Oid,
    Cloid,
}

struct OfficialValue<'a> {
    value: &'a Value,
    shape: Shape,
}

impl Serialize for OfficialValue<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.value {
            Value::Array(values) => serialize_array(values, self.shape, serializer),
            Value::Object(map) => serialize_object(map, self.shape, serializer),
            Value::Null => serializer.serialize_unit(),
            Value::Bool(value) => serializer.serialize_bool(*value),
            Value::Number(value) => serialize_number(value, serializer),
            Value::String(value) => serializer.serialize_str(value),
        }
    }
}

fn serialize_number<S>(number: &Number, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(value) = number.as_i64() {
        serializer.serialize_i64(value)
    } else if let Some(value) = number.as_u64() {
        serializer.serialize_u64(value)
    } else if let Some(value) = number.as_f64().filter(|value| value.is_finite()) {
        serializer.serialize_f64(value)
    } else {
        Err(S::Error::custom(
            "unsupported Hyperliquid action numeric value",
        ))
    }
}

fn serialize_array<S>(values: &[Value], shape: Shape, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let item_shape = match shape {
        Shape::Orders => Shape::Order,
        Shape::Cancels(CancelKind::Oid) => Shape::Cancel,
        Shape::Cancels(CancelKind::Cloid) => Shape::CancelByCloid,
        _ => return Err(S::Error::custom("unsupported Hyperliquid action array")),
    };
    let mut sequence = serializer.serialize_seq(Some(values.len()))?;
    for value in values {
        sequence.serialize_element(&OfficialValue {
            value,
            shape: item_shape,
        })?;
    }
    sequence.end()
}

fn serialize_object<S>(
    map: &Map<String, Value>,
    shape: Shape,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let schema = schema_for(map, shape).map_err(S::Error::custom)?;
    validate_schema(map, &schema).map_err(S::Error::custom)?;

    let mut output = serializer.serialize_map(Some(map.len()))?;
    for key in schema.allowed {
        if let Some(value) = map.get(*key) {
            output.serialize_entry(
                key,
                &OfficialValue {
                    value,
                    shape: child_shape(map, shape, key).map_err(S::Error::custom)?,
                },
            )?;
        }
    }
    output.end()
}

struct Schema {
    allowed: &'static [&'static str],
    required: &'static [&'static str],
}

fn schema_for(map: &Map<String, Value>, shape: Shape) -> Result<Schema, String> {
    let schema = match shape {
        Shape::Action => match action_type(map)? {
            "order" => Schema {
                allowed: ORDER_ACTION_KEYS,
                required: ORDER_ACTION_REQUIRED,
            },
            "cancel" | "cancelByCloid" => Schema {
                allowed: CANCEL_ACTION_KEYS,
                required: CANCEL_ACTION_KEYS,
            },
            "dummy" => Schema {
                allowed: DUMMY_KEYS,
                required: DUMMY_KEYS,
            },
            "noop" => Schema {
                allowed: TYPE_ONLY_KEYS,
                required: TYPE_ONLY_KEYS,
            },
            action_type => {
                return Err(format!(
                    "unsupported Hyperliquid L1 action type: {action_type}"
                ));
            }
        },
        Shape::Order => Schema {
            allowed: ORDER_KEYS,
            required: ORDER_REQUIRED,
        },
        Shape::OrderType => Schema {
            allowed: ORDER_TYPE_KEYS,
            required: &[],
        },
        Shape::Limit => Schema {
            allowed: LIMIT_KEYS,
            required: LIMIT_KEYS,
        },
        Shape::Trigger => Schema {
            allowed: TRIGGER_KEYS,
            required: TRIGGER_KEYS,
        },
        Shape::Cancel => Schema {
            allowed: CANCEL_KEYS,
            required: CANCEL_KEYS,
        },
        Shape::CancelByCloid => Schema {
            allowed: CANCEL_BY_CLOID_KEYS,
            required: CANCEL_BY_CLOID_KEYS,
        },
        Shape::Builder => Schema {
            allowed: BUILDER_KEYS,
            required: BUILDER_KEYS,
        },
        Shape::Grouping => Schema {
            allowed: GROUPING_KEYS,
            required: GROUPING_KEYS,
        },
        Shape::Orders | Shape::Cancels(_) | Shape::Scalar => {
            return Err("unexpected Hyperliquid action object".to_owned());
        }
    };
    if matches!(shape, Shape::OrderType) && map.len() != 1 {
        return Err("Hyperliquid order type must contain exactly one variant".to_owned());
    }
    Ok(schema)
}

fn validate_schema(map: &Map<String, Value>, schema: &Schema) -> Result<(), String> {
    if let Some(key) = map
        .keys()
        .find(|key| !schema.allowed.contains(&key.as_str()))
    {
        return Err(format!("unsupported Hyperliquid action field: {key}"));
    }
    if let Some(key) = schema.required.iter().find(|key| !map.contains_key(**key)) {
        return Err(format!("missing Hyperliquid action field: {key}"));
    }
    Ok(())
}

fn child_shape(map: &Map<String, Value>, shape: Shape, key: &str) -> Result<Shape, String> {
    let child = match (shape, key) {
        (Shape::Action, "orders") => Shape::Orders,
        (Shape::Action, "grouping") if map.get(key).is_some_and(Value::is_object) => {
            Shape::Grouping
        }
        (Shape::Action, "builder") => Shape::Builder,
        (Shape::Action, "cancels") => match action_type(map)? {
            "cancel" => Shape::Cancels(CancelKind::Oid),
            "cancelByCloid" => Shape::Cancels(CancelKind::Cloid),
            _ => return Err("cancels field belongs to an unsupported action".to_owned()),
        },
        (Shape::Order, "t") => Shape::OrderType,
        (Shape::OrderType, "limit") => Shape::Limit,
        (Shape::OrderType, "trigger") => Shape::Trigger,
        _ => Shape::Scalar,
    };
    Ok(child)
}

fn action_type(map: &Map<String, Value>) -> Result<&str, String> {
    map.get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Hyperliquid L1 action requires a string type".to_owned())
}
