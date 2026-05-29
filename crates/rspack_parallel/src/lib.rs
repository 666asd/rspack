mod iterator_consumer;
pub mod scope;

use std::{
  cell::UnsafeCell,
  future::Future,
  iter::ExactSizeIterator,
  mem::{ManuallyDrop, MaybeUninit},
};

pub use iterator_consumer::{FutureConsumer, TryFutureConsumer};
pub use scope::scope;

/// Spawn futures from an iterator and collect their outputs into a vec.
///
/// This is a wrapper around `scope`, but allows non-'static return values.
///
/// # Safety
///
/// Its safety assumptions are the same as `scope`.
///
/// # Example
///
/// ```rust
/// # #[tokio::test]
/// # async fn foo() {
/// async fn handle(s: &str) -> (usize, &str) {
///   (s.len(), s)
/// }
///
/// let data: Vec<String> = vec!["hello".into(), "world".into(), "!".into()];
/// let tasks = data.iter().map(|s| handle(s));
///
/// let list = unsafe { spawn_iter_then_collect(tasks).await };
///
/// assert_eq!(list, vec![(5, "hello"), (5, "world"), (1, "!")]);
/// # }
/// ```
pub async unsafe fn spawn_iter_then_collect<I, F, O>(iter: I) -> Vec<O>
where
  I: IntoIterator<Item = F>,
  I::IntoIter: ExactSizeIterator,
  F: Future<Output = O> + Send + Sync,
  O: Send + Sync,
{
  // TODO use `std::cell::SyncUnsafeCell`
  //
  // see https://github.com/rust-lang/rust/issues/95439
  #[repr(transparent)]
  struct SyncUnsafeCell<T: ?Sized>(UnsafeCell<T>);

  // # Safety
  //
  // We guarantee that `SyncUnsafeCell` will never be accesse parallel
  unsafe impl<T: ?Sized + Sync> Sync for SyncUnsafeCell<T> {}

  let iter = iter.into_iter();
  let output: Box<[MaybeUninit<SyncUnsafeCell<O>>]> = Box::new_uninit_slice(iter.len());

  scope(|token| {
    for (i, f) in iter.enumerate() {
      // # Safety
      //
      // The caller needs to ensure that the task is legally consumed
      let spawner = unsafe { token.used((f, &output)) };

      spawner.spawn(move |(f, output)| async move {
        let result = f.await;

        // # Safety
        //
        // This assumes that the length provided by the `ExactSizeIterator` is correct,
        // and will abort if it is not.
        let slot = &output[i];

        // # Safety
        //
        // because transparent repr
        let slot = slot.as_ptr().cast::<UnsafeCell<O>>();

        // # Safety
        //
        // This slot is exclusive to the thread and
        // will not be accessed by other threads at the same time.
        unsafe {
          UnsafeCell::raw_get(slot).write(result);
        }
      });
    }
  })
  .await;

  // # Safety
  //
  // `scope` ensures that all slots are initialized after completion
  let output = unsafe { output.assume_init() };
  let output = Vec::from(output);

  unsafe {
    // TODO use into_raw_parts
    //
    // see https://github.com/rust-lang/rust/issues/65816
    let mut output = ManuallyDrop::new(output);
    let ptr = output.as_mut_ptr();
    let len = output.len();
    let cap = output.capacity();

    // # Safety
    //
    // because transparent repr
    let ptr = ptr.cast::<O>();
    Vec::from_raw_parts(ptr, len, cap)
  }
}

/// Spawn fallible futures from an iterator, collect their outputs into a vec,
/// then return the first error if any task failed.
///
/// This keeps the work on the compiler Tokio runtime while preserving the
/// ordered collection shape commonly used by former parallel map/collect call
/// sites.
///
/// # Safety
///
/// Its safety assumptions are the same as [`scope`] and
/// [`spawn_iter_then_collect`].
pub async unsafe fn spawn_iter_then_try_collect<I, F, O, E>(iter: I) -> Result<Vec<O>, E>
where
  I: IntoIterator<Item = F>,
  I::IntoIter: ExactSizeIterator,
  F: Future<Output = Result<O, E>> + Send + Sync,
  O: Send + Sync,
  E: Send + Sync,
{
  let items = unsafe { spawn_iter_then_collect(iter).await };
  items.into_iter().collect()
}
