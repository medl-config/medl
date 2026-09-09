use indexmap::IndexMap;

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Map(IndexMap<String, Value>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
                (*a as f64) == *b
            }
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => a == b,
            _ => false,
        }
    }
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) | Value::Float(_) => "number",
            Value::Str(_) => "string",
            Value::List(_) => "list",
            Value::Map(_) => "map",
        }
    }

    /// How the value appears inside an interpolated string. `None` for null, list, map.
    pub fn interp_string(&self) -> Option<String> {
        match self {
            Value::Str(s) => Some(s.clone()),
            Value::Int(n) => Some(n.to_string()),
            Value::Float(f) => Some(f.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Null | Value::List(_) | Value::Map(_) => None,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Int(n) => serde_json::Value::from(*n),
            Value::Float(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::Str(s) => serde_json::Value::String(s.clone()),
            Value::List(items) => {
                serde_json::Value::Array(items.iter().map(Value::to_json).collect())
            }
            Value::Map(m) => {
                serde_json::Value::Object(m.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    #[test]
    fn ints_and_floats_compare_numerically() {
        assert_eq!(Value::Int(1), Value::Float(1.0));
        assert_ne!(Value::Int(1), Value::Float(1.5));
        assert_ne!(Value::Int(1), Value::Str("1".into()));
        assert_eq!(
            Value::List(vec![Value::Int(1)]),
            Value::List(vec![Value::Float(1.0)])
        );
    }

    #[test]
    fn interp_string_stringifies_scalars_only() {
        assert_eq!(Value::Str("a".into()).interp_string(), Some("a".into()));
        assert_eq!(Value::Int(30).interp_string(), Some("30".into()));
        assert_eq!(Value::Float(1.5).interp_string(), Some("1.5".into()));
        assert_eq!(Value::Bool(true).interp_string(), Some("true".into()));
        assert_eq!(Value::Null.interp_string(), None);
        assert_eq!(Value::List(vec![]).interp_string(), None);
        assert_eq!(Value::Map(IndexMap::new()).interp_string(), None);
    }

    #[test]
    fn type_names() {
        assert_eq!(Value::Null.type_name(), "null");
        assert_eq!(Value::Int(1).type_name(), "number");
        assert_eq!(Value::Float(1.0).type_name(), "number");
        assert_eq!(Value::Map(IndexMap::new()).type_name(), "map");
    }

    #[test]
    fn to_json_preserves_key_order() {
        let mut m = IndexMap::new();
        m.insert("z".to_string(), Value::Int(1));
        m.insert(
            "a".to_string(),
            Value::List(vec![Value::Bool(false), Value::Null, Value::Float(2.5)]),
        );
        let json = Value::Map(m).to_json();
        assert_eq!(
            serde_json::to_string(&json).unwrap(),
            r#"{"z":1,"a":[false,null,2.5]}"#
        );
    }
}
