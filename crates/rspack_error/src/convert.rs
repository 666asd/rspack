use crate::{
  Result,
  error::{Error, Label},
};

pub trait ToStringResultToRspackResultExt<T, E: ToString> {
  fn to_rspack_result(self) -> Result<T>;
  fn to_rspack_result_with_message(self, formatter: impl FnOnce(E) -> String) -> Result<T>;
}

impl<T, E: ToString> ToStringResultToRspackResultExt<T, E> for std::result::Result<T, E> {
  fn to_rspack_result(self) -> Result<T> {
    self.map_err(|e| crate::error!(e.to_string()))
  }
  fn to_rspack_result_with_message(self, formatter: impl FnOnce(E) -> String) -> Result<T> {
    self.map_err(|e| crate::error!(formatter(e)))
  }
}

pub trait SerdeResultToRspackResultExt<T> {
  fn to_rspack_result_with_detail(self, content: &str, msg: &str) -> Result<T>;
}

impl<T> SerdeResultToRspackResultExt<T> for std::result::Result<T, simd_json::Error> {
  fn to_rspack_result_with_detail(self, content: &str, msg: &str) -> Result<T> {
    self.map_err(|e| {
      let offset = e.index().min(content.len());
      let mut error = Error::error(msg.into());
      error.labels = Some(vec![Label {
        name: Some(e.to_string()),
        offset,
        len: 0,
      }]);
      error.src = Some(content.to_string());
      error
    })
  }
}

impl<T> SerdeResultToRspackResultExt<T> for std::result::Result<T, serde_json::Error> {
  fn to_rspack_result_with_detail(self, content: &str, msg: &str) -> Result<T> {
    self.map_err(|e| {
      let offset = serde_json_error_offset(content, e.line(), e.column());
      let mut error = Error::error(msg.into());
      error.labels = Some(vec![Label {
        name: Some(e.to_string()),
        offset,
        len: 0,
      }]);
      error.src = Some(content.to_string());
      error
    })
  }
}

fn serde_json_error_offset(content: &str, line: usize, column: usize) -> usize {
  if line == 0 {
    return content.len();
  }

  let bytes = content.as_bytes();
  let mut line_start = 0;
  for _ in 1..line {
    let Some(relative_newline) = bytes[line_start..].iter().position(|&b| b == b'\n') else {
      return content.len();
    };
    line_start += relative_newline + 1;
  }

  let line_end = bytes[line_start..]
    .iter()
    .position(|&b| b == b'\n')
    .map_or(content.len(), |relative_newline| {
      line_start + relative_newline
    });

  let mut offset = (line_start + column.saturating_sub(1)).min(line_end);
  while !content.is_char_boundary(offset) {
    offset = offset.saturating_sub(1);
  }
  offset
}

pub trait AnyhowResultToRspackResultExt<T> {
  fn to_rspack_result_from_anyhow(self) -> Result<T>;
}

impl<T> AnyhowResultToRspackResultExt<T> for std::result::Result<T, anyhow::Error> {
  fn to_rspack_result_from_anyhow(self) -> Result<T> {
    self.map_err(|e| e.into())
  }
}
