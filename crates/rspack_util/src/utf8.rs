use std::borrow::Cow;

#[inline]
pub fn from_utf8(bytes: &[u8]) -> Result<&str, simdutf8::basic::Utf8Error> {
  simdutf8::basic::from_utf8(bytes)
}

#[inline]
pub fn from_utf8_compat(bytes: &[u8]) -> Result<&str, simdutf8::compat::Utf8Error> {
  simdutf8::compat::from_utf8(bytes)
}

#[inline]
#[allow(unsafe_code)]
pub fn string_from_utf8(bytes: Vec<u8>) -> Result<String, simdutf8::basic::Utf8Error> {
  from_utf8(&bytes)?;
  // SAFETY: simdutf8 validated the buffer as UTF-8 above.
  Ok(unsafe { String::from_utf8_unchecked(bytes) })
}

#[inline]
#[allow(unsafe_code)]
pub fn string_from_utf8_compat(bytes: Vec<u8>) -> Result<String, simdutf8::compat::Utf8Error> {
  from_utf8_compat(&bytes)?;
  // SAFETY: simdutf8 validated the buffer as UTF-8 above.
  Ok(unsafe { String::from_utf8_unchecked(bytes) })
}

#[inline]
pub fn from_utf8_lossy(bytes: &[u8]) -> Cow<'_, str> {
  match from_utf8(bytes) {
    Ok(s) => Cow::Borrowed(s),
    Err(_) => String::from_utf8_lossy(bytes),
  }
}

#[inline]
#[allow(unsafe_code)]
pub fn string_from_utf8_lossy(bytes: Vec<u8>) -> String {
  if from_utf8(&bytes).is_ok() {
    // SAFETY: simdutf8 validated the buffer as UTF-8 above.
    unsafe { String::from_utf8_unchecked(bytes) }
  } else {
    String::from_utf8_lossy(&bytes).into_owned()
  }
}
