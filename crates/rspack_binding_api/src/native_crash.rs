//! Native crash reporter for fatal signals (SIGSEGV / SIGBUS / SIGILL).
//!
//! Why this exists: the Rust `panic` hook in [`crate::panic`] only fires for
//! `panic!`. Native signals — most commonly stack overflow, but also UB in C
//! deps and unaligned access — bypass it entirely. Users then see only
//! `Illegal instruction: 4` or `Segmentation fault` with no clue where it
//! happened, which is why <https://github.com/web-infra-dev/rspack/pull/14167>
//! (and bugs like it) are so hard to triage from issue reports alone.
//!
//! The installer below catches those signals, prints the signal number and a
//! raw-address backtrace to stderr, then re-raises the signal with the
//! default handler so the OS still writes its crash dump
//! (`~/Library/Logs/DiagnosticReports/` on macOS, `coredumpctl` on Linux).
//!
//! ## Async-signal-safety
//!
//! Signal handlers may only call async-signal-safe functions (POSIX
//! `signal-safety(7)`). The handler:
//!
//! * never allocates,
//! * writes via raw `libc::write` to fd 2,
//! * captures frames via `backtrace::trace` with `unresolved` symbols,
//! * runs on an alternate stack installed with `sigaltstack`, so a stack
//!   overflow can't blow the handler itself.
//!
//! Symbolication is *not* signal-safe and is deferred — the printed addresses
//! can be resolved post-mortem with `atos -o rspack.<target>.node <addr>` or
//! `addr2line`.
//!
//! ## Per-thread alt stacks
//!
//! `sigaction` is process-wide but `sigaltstack` is per-thread. The installer
//! is therefore split into two:
//!
//! * [`install_native_crash_handler`] — registers the signal handlers
//!   process-wide and seeds an alt stack for the calling thread. Call once at
//!   module load.
//! * [`install_alt_stack_for_current_thread`] — idempotent per-thread; call
//!   from every worker thread (tokio / napi / rayon) so a stack-overflow
//!   crash on a worker can still run the handler on a fresh stack.

#[cfg(all(unix, not(target_family = "wasm")))]
mod imp {
  use std::{
    cell::Cell,
    sync::{
      Once,
      atomic::{AtomicBool, Ordering},
    },
  };

  // Alt-stack size. `SIGSTKSZ` would be ideal but it's not constant on Linux.
  // 64 KiB is the size `backtrace-on-stack-overflow` uses and matches Go's
  // `g0` alt-stack.
  const ALT_STACK_SIZE: usize = 64 * 1024;

  // Tracks whether the current thread already installed its alt stack so
  // repeated calls from `on_thread_start` hooks are cheap no-ops.
  thread_local! {
    static ALT_STACK_INSTALLED: Cell<bool> = const { Cell::new(false) };
  }

  // Prevent re-entrant handler if the handler itself faults.
  static IN_HANDLER: AtomicBool = AtomicBool::new(false);

  /// Process-wide install: register the signal handlers once and seed an alt
  /// stack for the calling thread. Worker threads must additionally call
  /// [`install_alt_stack_for_current_thread`].
  pub fn install() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
      install_signal(libc::SIGSEGV);
      install_signal(libc::SIGBUS);
      install_signal(libc::SIGILL);
    });
    install_alt_stack_for_current_thread();
  }

  /// Idempotent per-thread alt-stack installer.
  ///
  /// `sigaltstack` is per-thread — the kernel stores the (sp, size) pair per
  /// task — so every worker thread needs its own buffer. We leak one Box per
  /// thread; the buffer must outlive the thread because the kernel may dump
  /// a backtrace there during signal delivery.
  pub fn install_alt_stack_for_current_thread() {
    if ALT_STACK_INSTALLED.with(|c| c.replace(true)) {
      return;
    }
    // Leaked intentionally: see doc-comment above.
    let buf: &'static mut [u8] = Box::leak(vec![0u8; ALT_STACK_SIZE].into_boxed_slice());
    // SAFETY: `buf` is a fresh allocation we just produced; sigaltstack with
    // a non-null first arg + null second is a simple set.
    unsafe {
      let stack = libc::stack_t {
        ss_sp: buf.as_mut_ptr().cast(),
        ss_flags: 0,
        ss_size: ALT_STACK_SIZE,
      };
      // Failure is non-fatal — without an alt stack a stack-overflow signal
      // will still be unrecoverable, which is today's behavior.
      libc::sigaltstack(&stack, std::ptr::null_mut());
    }
  }

  fn install_signal(sig: libc::c_int) {
    // SAFETY: `sigaction` is the canonical way to install a handler; we
    // zero-init the struct first and the `extern "C"` handler matches the
    // expected ABI.
    unsafe {
      let mut sa: libc::sigaction = std::mem::zeroed();
      sa.sa_sigaction = handler as *const () as libc::sighandler_t;
      sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
      libc::sigemptyset(&mut sa.sa_mask);
      libc::sigaction(sig, &sa, std::ptr::null_mut());
    }
  }

  /// Raw signal handler. Must stay async-signal-safe.
  extern "C" fn handler(
    sig: libc::c_int,
    info: *mut libc::siginfo_t,
    _ucontext: *mut libc::c_void,
  ) {
    if IN_HANDLER.swap(true, Ordering::SeqCst) {
      // Already handling — re-raise with default and abort.
      restore_default_and_reraise(sig);
      return;
    }

    write_str(b"\n========================================\n");
    write_str(b"rspack native crash: signal=");
    write_signal_name(sig);
    // si_addr is the faulting address for SIGSEGV/SIGBUS — an address near
    // the top of the thread stack strongly implies stack overflow.
    if !info.is_null() {
      // SAFETY: signal handlers receive a valid siginfo_t when SA_SIGINFO is
      // set; reading si_addr is a simple field read.
      let addr = unsafe { (*info).si_addr() } as usize;
      write_str(b"  fault_addr=");
      write_hex(addr);
    }
    write_str(b"\nThis is a bug in rspack (or a native dep). Please file an issue: ");
    write_str(b"https://github.com/web-infra-dev/rspack/issues\n");
    write_str(b"Raw backtrace (resolve with `atos -o <binary> <addr>`):\n");

    // `backtrace::trace` is *mostly* async-signal-safe on Unix — it uses
    // libunwind's `_Unwind_Backtrace` which doesn't allocate. We intentionally
    // skip the `resolve` step because symbol resolution opens files and is
    // not signal-safe.
    let mut idx = 0u32;
    backtrace::trace(|frame| {
      write_str(b"  #");
      write_u32(idx);
      write_str(b"  ip=");
      write_hex(frame.ip() as usize);
      write_str(b"  sp=");
      write_hex(frame.sp() as usize);
      write_str(b"\n");
      idx = idx.wrapping_add(1);
      // Cap at a reasonable depth in case we're in a runaway recursion.
      idx < 96
    });

    write_str(b"========================================\n\n");

    // Reset to default and re-raise so the OS still produces a crash dump.
    restore_default_and_reraise(sig);
  }

  fn restore_default_and_reraise(sig: libc::c_int) {
    // SAFETY: same as install_signal, plus libc::raise is safe to call from
    // a signal handler per POSIX signal-safety(7).
    unsafe {
      let mut sa: libc::sigaction = std::mem::zeroed();
      sa.sa_sigaction = libc::SIG_DFL;
      sa.sa_flags = 0;
      libc::sigemptyset(&mut sa.sa_mask);
      libc::sigaction(sig, &sa, std::ptr::null_mut());
      libc::raise(sig);
    }
  }

  // --- async-signal-safe stderr writers ---

  fn write_str(s: &[u8]) {
    // Use libc::write directly — std::io::stderr().write_all goes through a
    // mutex which is unsafe in signal context.
    let mut written = 0usize;
    while written < s.len() {
      let n = unsafe {
        libc::write(
          libc::STDERR_FILENO,
          s.as_ptr().add(written).cast(),
          s.len() - written,
        )
      };
      if n <= 0 {
        return;
      }
      written += n as usize;
    }
  }

  fn write_signal_name(sig: libc::c_int) {
    let name: &[u8] = match sig {
      libc::SIGSEGV => b"SIGSEGV (likely stack overflow or null deref)",
      libc::SIGBUS => b"SIGBUS (likely misaligned access)",
      libc::SIGILL => b"SIGILL (likely Rust trap from stack-overflow guard)",
      _ => b"unknown",
    };
    write_str(name);
  }

  fn write_u32(mut n: u32) {
    let mut buf = [0u8; 10];
    let mut i = buf.len();
    if n == 0 {
      write_str(b"0");
      return;
    }
    while n > 0 {
      i -= 1;
      buf[i] = b'0' + (n % 10) as u8;
      n /= 10;
    }
    write_str(&buf[i..]);
  }

  fn write_hex(mut n: usize) {
    // 16 hex digits is enough for a 64-bit pointer; plus "0x" prefix.
    let mut buf = [0u8; 18];
    let mut i = buf.len();
    if n == 0 {
      i -= 1;
      buf[i] = b'0';
    } else {
      while n > 0 {
        i -= 1;
        let d = (n & 0xf) as u8;
        buf[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        n >>= 4;
      }
    }
    i -= 1;
    buf[i] = b'x';
    i -= 1;
    buf[i] = b'0';
    write_str(&buf[i..]);
  }
}

#[cfg(all(unix, not(target_family = "wasm")))]
pub use imp::{install as install_native_crash_handler, install_alt_stack_for_current_thread};

#[cfg(not(all(unix, not(target_family = "wasm"))))]
pub fn install_native_crash_handler() {
  // Windows uses structured exception handling and the napi-rs harness
  // already integrates with it; WASM has no signals. No-op here.
}

#[cfg(not(all(unix, not(target_family = "wasm"))))]
pub fn install_alt_stack_for_current_thread() {
  // No-op on Windows / WASM — see install_native_crash_handler.
}
