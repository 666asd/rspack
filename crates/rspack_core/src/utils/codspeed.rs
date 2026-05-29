/// Configure parallel work for CodSpeed runs.
///
/// Rspack no longer configures a separate global CPU pool for CodSpeed. Keep
/// this hook as a no-op for the existing CodSpeed setup group.
pub fn configure_current_thread_for_codspeed() {}
