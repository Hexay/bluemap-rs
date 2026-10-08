//! serde `Deserializer` over a [`Value`] that behaves like Configurate's object mapper: missing and `null` keys keep
//! the field default, unknown keys are ignored, scalars coerce leniently, a single value fills a list, and a
//! non-object for a nested section leaves its defaults.

pub(crate) mod coerce;
pub(crate) mod fields;
pub(crate) mod size;
mod value;

use serde::de::{self, DeserializeOwned, DeserializeSeed, IntoDeserializer, Visitor};

use crate::value::Value;

/// Mapping failure with the key path it occurred at.
#[derive(Debug, Clone, PartialEq)]
pub struct DeError {
    pub path: Vec<String>,
    pub message: String,
}

impl DeError {
    pub fn key(&self) -> String {
        self.path.join(".")
    }

    fn at(mut self, key: String) -> Self {
        self.path.insert(0, key);
        self
    }
}

impl std::fmt::Display for DeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.path.is_empty() {
            true => f.write_str(&self.message),
            false => write!(f, "{}: {}", self.key(), self.message),
        }
    }
}

impl std::error::Error for DeError {}

impl de::Error for DeError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        DeError { path: vec![], message: msg.to_string() }
    }
}

pub fn from_value<T: DeserializeOwned>(value: &Value) -> Result<T, DeError> {
    T::deserialize(ValueDe(value))
}

fn lift<T>(r: Result<T, String>) -> Result<T, DeError> {
    r.map_err(|message| DeError { path: vec![], message })
}

#[derive(Clone, Copy)]
pub(crate) struct ValueDe<'a>(pub &'a Value);

macro_rules! int_method {
    ($method:ident, $visit:ident, $ty:ty, $what:literal) => {
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
            let n = lift(coerce::to_int(self.0, <$ty>::MIN as i64, <$ty>::MAX as i64, $what))?;
            visitor.$visit(n as $ty)
        }
    };
}

impl<'de> de::Deserializer<'de> for ValueDe<'de> {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.0 {
            Value::Null => visitor.visit_unit(),
            Value::Bool(b) => visitor.visit_bool(*b),
            Value::Int(i) => visitor.visit_i64(*i),
            Value::Float(f) => visitor.visit_f64(*f),
            Value::String(s) => visitor.visit_str(s),
            Value::List(items) => visitor.visit_seq(Seq { items: items.iter(), index: 0 }),
            Value::Object(_) => self.deserialize_map(visitor),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_bool(lift(coerce::to_bool(self.0))?)
    }

    int_method!(deserialize_i8, visit_i8, i8, "a byte");
    int_method!(deserialize_i16, visit_i16, i16, "a short");
    int_method!(deserialize_i32, visit_i32, i32, "an integer");
    int_method!(deserialize_i64, visit_i64, i64, "a long");
    int_method!(deserialize_u8, visit_u8, u8, "an unsigned byte");
    int_method!(deserialize_u16, visit_u16, u16, "an unsigned short");
    int_method!(deserialize_u32, visit_u32, u32, "an unsigned integer");

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_u64(lift(coerce::to_int(self.0, 0, i64::MAX, "an unsigned long"))? as u64)
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_f32(lift(coerce::to_f32(self.0))?)
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_f64(lift(coerce::to_f64(self.0))?)
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_string(visitor)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_string(visitor)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_string(lift(coerce::to_string(self.0))?)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_string(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_string(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.0 {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(self, _: &'static str, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _: &'static str, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_newtype_struct(self)
    }

    /// A list maps element-wise; anything else is a one-element list (Configurate wraps single values).
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.0 {
            Value::List(items) => visitor.visit_seq(Seq { items: items.iter(), index: 0 }),
            single => visitor.visit_seq(Seq { items: std::slice::from_ref(single).iter(), index: 0 }),
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: usize,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.0 {
            Value::Object(map) => visitor.visit_map(Fields::new(map.iter())),
            other => Err(de::Error::custom(format!("expected an object {{ ... }}, got {}", other.type_name()))),
        }
    }

    /// Configurate's object mapper reads each field from a child node; a non-object has none, so all defaults.
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        match self.0 {
            Value::Object(map) => visitor.visit_map(Fields::new(map.iter())),
            _ => visitor.visit_map(Fields::new(crate::value::EMPTY_MAP.iter())),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        let s = lift(coerce::to_string(self.0))?;
        visitor.visit_enum(s.into_deserializer())
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_string(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_unit()
    }
}

struct Seq<'a> {
    items: std::slice::Iter<'a, Value>,
    index: usize,
}

impl<'de> de::SeqAccess<'de> for Seq<'de> {
    type Error = DeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, DeError> {
        let Some(item) = self.items.next() else { return Ok(None) };
        let index = self.index;
        self.index += 1;
        seed.deserialize(ValueDe(item)).map(Some).map_err(|e| e.at(index.to_string()))
    }
}

/// Object members, skipping `null` ones (Configurate drops them on load).
struct Fields<'a, I: Iterator<Item = (&'a str, &'a Value)>> {
    iter: I,
    current: Option<(&'a str, &'a Value)>,
}

impl<'a, I: Iterator<Item = (&'a str, &'a Value)>> Fields<'a, I> {
    fn new(iter: I) -> Self {
        Self { iter, current: None }
    }
}

impl<'de, I: Iterator<Item = (&'de str, &'de Value)>> de::MapAccess<'de> for Fields<'de, I> {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, DeError> {
        self.current = self.iter.by_ref().find(|(_, v)| !matches!(v, Value::Null));
        match self.current {
            Some((key, _)) => seed.deserialize(key.into_deserializer()).map(Some),
            None => Ok(None),
        }
    }

    fn next_value_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<T::Value, DeError> {
        let (key, value) = self.current.take().expect("serde calls next_key first");
        seed.deserialize(ValueDe(value)).map_err(|e| e.at(key.to_owned()))
    }
}
