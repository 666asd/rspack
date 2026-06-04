use napi::{
  Env, JsValue,
  bindgen_prelude::{
    Array, FromNapiValue, JsObjectValue, Null, Object, ToNapiValue, TypeName, Unknown,
    ValidateNapiValue,
  },
  sys,
};
use simd_json::{OwnedValue, StaticNode};

#[derive(Debug, Clone, PartialEq)]
pub struct JsonValue(pub OwnedValue);

impl JsonValue {
  pub fn into_inner(self) -> OwnedValue {
    self.0
  }

  pub fn as_inner(&self) -> &OwnedValue {
    &self.0
  }
}

impl From<OwnedValue> for JsonValue {
  fn from(value: OwnedValue) -> Self {
    Self(value)
  }
}

impl From<JsonValue> for OwnedValue {
  fn from(value: JsonValue) -> Self {
    value.0
  }
}

impl std::ops::Deref for JsonValue {
  type Target = OwnedValue;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl std::hash::Hash for JsonValue {
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    self.0.to_string().hash(state);
  }
}

impl TypeName for JsonValue {
  fn type_name() -> &'static str {
    "JsonValue"
  }

  fn value_type() -> napi::ValueType {
    napi::ValueType::Unknown
  }
}

impl ValidateNapiValue for JsonValue {}

impl FromNapiValue for JsonValue {
  unsafe fn from_napi_value(env: sys::napi_env, napi_val: sys::napi_value) -> napi::Result<Self> {
    let unknown = unsafe { Unknown::from_napi_value(env, napi_val)? };
    unknown_to_json_value(unknown)?
      .map(Self)
      .ok_or_else(|| napi::Error::from_reason("Unsupported value for JSON conversion"))
  }
}

impl ToNapiValue for JsonValue {
  unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> napi::Result<sys::napi_value> {
    unsafe { json_value_to_napi_value(env, &val.0) }
  }
}

impl ToNapiValue for &JsonValue {
  unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> napi::Result<sys::napi_value> {
    unsafe { json_value_to_napi_value(env, &val.0) }
  }
}

pub fn downcast_into<T: FromNapiValue + 'static>(o: Unknown) -> napi::Result<T> {
  <T as FromNapiValue>::from_unknown(o)
}

pub fn object_assign(target: &mut Object, source: &Object) -> napi::Result<()> {
  let names = source.get_all_property_names(
    napi::KeyCollectionMode::OwnOnly,
    napi::KeyFilter::AllProperties,
    napi::KeyConversion::KeepNumbers,
  )?;
  let names = Array::from_unknown(names.to_unknown())?;

  for index in 0..names.len() {
    if let Some(name) = names.get::<Unknown>(index)? {
      let value = source.get_property::<Unknown, Unknown>(name)?;
      target.set_property::<Unknown, Unknown>(name, value)?;
    }
  }

  Ok(())
}

pub fn object_clone<'a>(env: &Env, object: &'a Object<'a>) -> napi::Result<Object<'a>> {
  let mut new_object = Object::new(env)?;

  let names = object.get_all_property_names(
    napi::KeyCollectionMode::OwnOnly,
    napi::KeyFilter::AllProperties,
    napi::KeyConversion::KeepNumbers,
  )?;
  let names = Array::from_unknown(names.to_unknown())?;

  for index in 0..names.len() {
    if let Some(name) = names.get::<Unknown>(index)? {
      let value = object.get_property::<Unknown, Unknown>(name)?;
      new_object.set_property::<Unknown, Unknown>(name, value)?;
    }
  }

  Ok(new_object)
}

/// Converts an owned JSON value into a JavaScript value.
///
/// # Safety
///
/// `env` must be a valid N-API environment for the current call scope.
pub unsafe fn json_value_to_napi_value(
  env: sys::napi_env,
  value: &OwnedValue,
) -> napi::Result<sys::napi_value> {
  match value {
    OwnedValue::Static(StaticNode::Null) => unsafe { Null::to_napi_value(env, Null) },
    OwnedValue::Static(StaticNode::Bool(b)) => unsafe { bool::to_napi_value(env, *b) },
    OwnedValue::Static(StaticNode::I64(n)) => unsafe { i64::to_napi_value(env, *n) },
    OwnedValue::Static(StaticNode::U64(n)) => {
      if let Ok(n) = u32::try_from(*n) {
        unsafe { u32::to_napi_value(env, n) }
      } else {
        unsafe { f64::to_napi_value(env, *n as f64) }
      }
    }
    OwnedValue::Static(StaticNode::F64(n)) => {
      #[allow(clippy::useless_conversion)]
      let n: f64 = (*n).into();
      unsafe { f64::to_napi_value(env, n) }
    }
    OwnedValue::String(s) => unsafe { String::to_napi_value(env, s.clone()) },
    OwnedValue::Array(array) => {
      let array = array.iter().cloned().map(JsonValue).collect::<Vec<_>>();
      unsafe { Vec::<JsonValue>::to_napi_value(env, array) }
    }
    OwnedValue::Object(object) => {
      let mut js_object = Object::new(&Env::from(env))?;
      for (key, value) in object.iter() {
        js_object.set(key, JsonValue(value.clone()))?;
      }
      unsafe { Object::to_napi_value(env, js_object) }
    }
  }
}

pub fn unknown_to_json_value(value: Unknown) -> napi::Result<Option<simd_json::OwnedValue>> {
  if value.is_array()? {
    let js_array = Array::from_unknown(value)?;
    let mut array = Vec::with_capacity(js_array.len() as usize);

    for index in 0..js_array.len() {
      if let Some(item) = js_array.get::<Unknown>(index)? {
        if let Some(json_val) = unknown_to_json_value(item)? {
          array.push(json_val);
        } else {
          array.push(().into());
        }
      } else {
        array.push(().into());
      }
    }

    return Ok(Some(array.into()));
  }

  match value.get_type()? {
    napi::ValueType::Null => Ok(Some(().into())),
    napi::ValueType::Boolean => {
      let b = value.coerce_to_bool()?;
      Ok(Some(b.into()))
    }
    napi::ValueType::Number => {
      let number = value.coerce_to_number()?.get_double()?;
      if number.is_finite() {
        Ok(Some(number.into()))
      } else {
        Ok(None)
      }
    }
    napi::ValueType::String => {
      let s = value.coerce_to_string()?.into_utf8()?.into_owned()?;
      Ok(Some(simd_json::OwnedValue::String(s)))
    }
    napi::ValueType::Object => {
      let object = value.coerce_to_object()?;
      let mut map = simd_json::value::owned::Object::default();

      let names = Array::from_unknown(object.get_property_names()?.to_unknown())?;
      for index in 0..names.len() {
        if let Some(name) = names.get::<String>(index)? {
          let prop_val = object.get_named_property::<Unknown>(&name)?;
          if let Some(json_val) = unknown_to_json_value(prop_val)? {
            map.insert(name, json_val);
          }
        }
      }

      Ok(Some(map.into()))
    }
    napi::ValueType::Undefined
    | napi::ValueType::Symbol
    | napi::ValueType::Function
    | napi::ValueType::External
    | napi::ValueType::BigInt
    | napi::ValueType::Unknown => Ok(None),
  }
}
