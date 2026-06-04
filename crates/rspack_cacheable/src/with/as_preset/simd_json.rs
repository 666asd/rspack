use rkyv::{
  Place,
  rancor::Fallible,
  ser::Writer,
  string::{ArchivedString, StringResolver},
  with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use simd_json::{OwnedValue as Value, value::owned::Object as Map};

use super::AsPreset;
use crate::{Error, Result};

pub struct SerdeJsonResolver {
  inner: StringResolver,
  value: String,
}

// for Value
impl ArchiveWith<Value> for AsPreset {
  type Archived = ArchivedString;
  type Resolver = SerdeJsonResolver;

  #[inline]
  fn resolve_with(_field: &Value, resolver: Self::Resolver, out: Place<Self::Archived>) {
    let SerdeJsonResolver { inner, value } = resolver;
    ArchivedString::resolve_from_str(&value, inner, out);
  }
}

impl<S> SerializeWith<Value, S> for AsPreset
where
  S: Fallible<Error = Error> + Writer,
{
  #[inline]
  fn serialize_with(field: &Value, serializer: &mut S) -> Result<Self::Resolver> {
    let value = simd_json::to_string(field)
      .map_err(|_| Error::MessageError("serialize simd_json value failed"))?;
    let inner = ArchivedString::serialize_from_str(&value, serializer)?;
    Ok(SerdeJsonResolver { value, inner })
  }
}

impl<D> DeserializeWith<ArchivedString, Value, D> for AsPreset
where
  D: Fallible<Error = Error>,
{
  #[inline]
  fn deserialize_with(field: &ArchivedString, _: &mut D) -> Result<Value> {
    simd_json::from_reader(field.as_str().as_bytes())
      .map_err(|_| Error::MessageError("deserialize simd_json value failed"))
  }
}

// for Object
impl ArchiveWith<Map> for AsPreset {
  type Archived = ArchivedString;
  type Resolver = SerdeJsonResolver;

  #[inline]
  fn resolve_with(_field: &Map, resolver: Self::Resolver, out: Place<Self::Archived>) {
    let SerdeJsonResolver { inner, value } = resolver;
    ArchivedString::resolve_from_str(&value, inner, out);
  }
}

impl<S> SerializeWith<Map, S> for AsPreset
where
  S: Fallible<Error = Error> + Writer,
{
  #[inline]
  fn serialize_with(field: &Map, serializer: &mut S) -> Result<Self::Resolver> {
    let value = simd_json::to_string(field)
      .map_err(|_| Error::MessageError("serialize simd_json value failed"))?;
    let inner = ArchivedString::serialize_from_str(&value, serializer)?;
    Ok(SerdeJsonResolver { value, inner })
  }
}

impl<D> DeserializeWith<ArchivedString, Map, D> for AsPreset
where
  D: Fallible<Error = Error>,
{
  #[inline]
  fn deserialize_with(field: &ArchivedString, _: &mut D) -> Result<Map> {
    simd_json::from_reader(field.as_str().as_bytes())
      .map_err(|_| Error::MessageError("deserialize simd_json value failed"))
  }
}
