use std::{
  collections::HashMap,
  ffi::c_void,
  fmt,
  path::Path,
  ptr,
  sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicU32, Ordering},
    mpsc,
  },
  thread::ThreadId,
};

use anyhow::{Context, anyhow};
use napi::{
  Env, JsValue, Status,
  bindgen_prelude::{ArrayBuffer, Buffer, FnArgs, FromNapiValue, Function, JsObjectValue, Object},
  sys,
};
use rspack_util::swc::runtime as rspack_swc_runtime;
use swc_plugin_runner::runtime;

const RUNTIME_IDENTIFIER: &str = "node-webassembly-v8-v1";

static NODE_WASM_HOST: OnceLock<Arc<NodeWasmHost>> = OnceLock::new();

#[napi(js_name = "__registerNodeWasmRuntime")]
pub fn register_node_wasm_runtime(env: Env, helper: Object) -> napi::Result<()> {
  let host = if let Some(host) = NODE_WASM_HOST.get() {
    Arc::clone(host)
  } else {
    let host = NodeWasmHost::new(env, helper)?;
    let _ = NODE_WASM_HOST.set(Arc::clone(&host));
    host
  };

  rspack_swc_runtime::set_plugin_runtime(Arc::new(NodeWasmRuntime { host }));
  Ok(())
}

#[napi(js_name = "__nodeWasmImportCall")]
pub fn node_wasm_import_call(
  env: Env,
  instance_id: u32,
  import_index: u32,
  args: Vec<i32>,
  mut memory_buffer: ArrayBuffer,
) -> napi::Result<Vec<i32>> {
  let host = NODE_WASM_HOST.get().ok_or_else(|| {
    napi::Error::from_reason("Node.js WebAssembly runtime has not been registered")
  })?;

  let imports = host
    .instances
    .lock()
    .expect("node wasm imports should not be poisoned")
    .get(&instance_id)
    .cloned()
    .ok_or_else(|| {
      napi::Error::from_reason(format!(
        "SWC Wasm plugin instance {instance_id} no longer exists"
      ))
    })?;

  let func = imports.imports.get(import_index as usize).ok_or_else(|| {
    napi::Error::from_reason(format!(
      "SWC Wasm plugin import {import_index} does not exist for instance {instance_id}"
    ))
  })?;

  if args.len() != func.sign.0 as usize {
    return Err(napi::Error::from_reason(format!(
      "SWC Wasm plugin import {import_index} expected {} arguments, received {}",
      func.sign.0,
      args.len()
    )));
  }

  let mut output = vec![0; func.sign.1 as usize];
  let direct_memory = {
    let memory = unsafe { memory_buffer.as_mut() };
    DirectMemory {
      ptr: memory.as_mut_ptr(),
      len: memory.len(),
    }
  };
  let mut caller = NodeWasmCaller {
    host: Arc::clone(host),
    instance_id,
    direct_env: Some(env.raw()),
    direct_memory: Some(direct_memory),
  };
  (func.func)(&mut caller, &args, &mut output);
  Ok(output)
}

struct NodeWasmHost {
  env: sys::napi_env,
  js_thread_id: ThreadId,
  helper_ref: sys::napi_ref,
  task_tsfn: sys::napi_threadsafe_function,
  next_module_id: AtomicU32,
  next_instance_id: AtomicU32,
  instances: Mutex<HashMap<u32, Arc<InstanceImports>>>,
}

unsafe impl Send for NodeWasmHost {}
unsafe impl Sync for NodeWasmHost {}

impl NodeWasmHost {
  fn new(env: Env, helper: Object) -> napi::Result<Arc<Self>> {
    let raw_env = env.raw();
    let mut helper_ref = ptr::null_mut();
    check_napi_status(
      unsafe { sys::napi_create_reference(raw_env, helper.raw(), 1, &mut helper_ref) },
      "Failed to create Node.js WebAssembly runtime helper reference",
    )?;

    let mut async_resource_name = ptr::null_mut();
    let name = c"rspack_node_wasm_runtime";
    check_napi_status(
      unsafe {
        sys::napi_create_string_utf8(
          raw_env,
          name.as_ptr(),
          name.to_bytes().len() as isize,
          &mut async_resource_name,
        )
      },
      "Failed to create Node.js WebAssembly runtime async resource name",
    )?;

    let mut task_tsfn = ptr::null_mut();
    check_napi_status(
      unsafe {
        sys::napi_create_threadsafe_function(
          raw_env,
          ptr::null_mut(),
          ptr::null_mut(),
          async_resource_name,
          0,
          1,
          ptr::null_mut(),
          None,
          ptr::null_mut(),
          Some(node_wasm_task_callback),
          &mut task_tsfn,
        )
      },
      "Failed to create Node.js WebAssembly runtime threadsafe function",
    )?;
    check_napi_status(
      unsafe { sys::napi_unref_threadsafe_function(raw_env, task_tsfn) },
      "Failed to unref Node.js WebAssembly runtime threadsafe function",
    )?;

    Ok(Arc::new(Self {
      env: raw_env,
      js_thread_id: std::thread::current().id(),
      helper_ref,
      task_tsfn,
      next_module_id: AtomicU32::new(1),
      next_instance_id: AtomicU32::new(1),
      instances: Default::default(),
    }))
  }

  fn next_module_id(&self) -> u32 {
    self.next_module_id.fetch_add(1, Ordering::Relaxed)
  }

  fn next_instance_id(&self) -> u32 {
    self.next_instance_id.fetch_add(1, Ordering::Relaxed)
  }
}

struct InstanceImports {
  imports: Vec<runtime::Func>,
}

#[derive(Clone)]
struct NodeWasmRuntime {
  host: Arc<NodeWasmHost>,
}

impl fmt::Debug for NodeWasmRuntime {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("NodeWasmRuntime").finish()
  }
}

impl runtime::Runtime for NodeWasmRuntime {
  fn identifier(&self) -> &'static str {
    RUNTIME_IDENTIFIER
  }

  fn prepare_module(&self, bytes: &[u8]) -> anyhow::Result<runtime::ModuleCache> {
    let module = compile_module(&self.host, bytes.to_vec())?;
    Ok(runtime::ModuleCache(Box::new(NodeWasmCache { module })))
  }

  fn clone_cache(&self, cache: &runtime::ModuleCache) -> Option<runtime::ModuleCache> {
    let cache = cache.0.downcast_ref::<NodeWasmCache>()?;
    Some(runtime::ModuleCache(Box::new(NodeWasmCache {
      module: Arc::clone(&cache.module),
    })))
  }

  unsafe fn load_cache(&self, _path: &Path) -> Option<runtime::ModuleCache> {
    None
  }

  fn store_cache(&self, _path: &Path, _cache: &runtime::ModuleCache) -> anyhow::Result<()> {
    // V8 WebAssembly.Module objects are not serialized to Rspack's filesystem
    // cache. We still use in-memory compiled modules through `prepare_module`.
    Ok(())
  }

  fn init(
    &self,
    _name: &str,
    imports: Vec<(String, runtime::Func)>,
    envs: Vec<(String, String)>,
    module: runtime::Module,
  ) -> anyhow::Result<Box<dyn runtime::Instance>> {
    let module = match module {
      runtime::Module::Cache(cache) => {
        let cache = cache
          .0
          .downcast::<NodeWasmCache>()
          .map_err(|_| anyhow!("invalid SWC Wasm module cache for Node.js runtime"))?;
        cache.module
      }
      runtime::Module::Bytes(bytes) => compile_module(&self.host, bytes.into_vec())?,
    };

    let instance_id = self.host.next_instance_id();
    let import_names = imports
      .iter()
      .map(|(name, _)| name.clone())
      .collect::<Vec<_>>();
    let imports = Arc::new(InstanceImports {
      imports: imports.into_iter().map(|(_, func)| func).collect(),
    });

    self
      .host
      .instances
      .lock()
      .expect("node wasm imports should not be poisoned")
      .insert(instance_id, imports);

    let envs_json = serde_json::to_string(&envs)?;
    if let Err(error) = run_js(
      &self.host,
      JsOp::Instantiate {
        instance_id,
        module_id: module.id,
        import_names,
        envs_json,
      },
    ) {
      self
        .host
        .instances
        .lock()
        .expect("node wasm imports should not be poisoned")
        .remove(&instance_id);
      return Err(error);
    }

    Ok(Box::new(NodeWasmInstance {
      host: Arc::clone(&self.host),
      instance_id,
      module,
    }))
  }
}

fn compile_module(host: &Arc<NodeWasmHost>, bytes: Vec<u8>) -> anyhow::Result<Arc<NodeWasmModule>> {
  let module_id = host.next_module_id();
  run_js(host, JsOp::Compile { module_id, bytes })?;
  Ok(Arc::new(NodeWasmModule {
    id: module_id,
    host: Arc::clone(host),
  }))
}

struct NodeWasmModule {
  id: u32,
  host: Arc<NodeWasmHost>,
}

impl Drop for NodeWasmModule {
  fn drop(&mut self) {
    let _ = run_js(&self.host, JsOp::DropModule { module_id: self.id });
  }
}

struct NodeWasmCache {
  module: Arc<NodeWasmModule>,
}

struct NodeWasmInstance {
  host: Arc<NodeWasmHost>,
  instance_id: u32,
  #[allow(dead_code)]
  module: Arc<NodeWasmModule>,
}

impl Drop for NodeWasmInstance {
  fn drop(&mut self) {
    self
      .host
      .instances
      .lock()
      .expect("node wasm imports should not be poisoned")
      .remove(&self.instance_id);
    let _ = run_js(
      &self.host,
      JsOp::DropInstance {
        instance_id: self.instance_id,
      },
    );
  }
}

impl runtime::Instance for NodeWasmInstance {
  fn transform(
    &mut self,
    program_ptr: u32,
    program_len: u32,
    unresolved_mark: u32,
    should_enable_comments_proxy: u32,
  ) -> anyhow::Result<u32> {
    match run_js(
      &self.host,
      JsOp::Transform {
        instance_id: self.instance_id,
        program_ptr,
        program_len,
        unresolved_mark,
        should_enable_comments_proxy,
      },
    )? {
      JsResult::U32(value) => Ok(value),
      _ => unreachable!("transform must return u32"),
    }
  }

  fn caller(&mut self) -> anyhow::Result<Box<dyn runtime::Caller<'_> + '_>> {
    Ok(Box::new(NodeWasmCaller {
      host: Arc::clone(&self.host),
      instance_id: self.instance_id,
      direct_env: None,
      direct_memory: None,
    }))
  }

  fn cache(&self) -> Option<runtime::ModuleCache> {
    Some(runtime::ModuleCache(Box::new(NodeWasmCache {
      module: Arc::clone(&self.module),
    })))
  }
}

struct NodeWasmCaller {
  host: Arc<NodeWasmHost>,
  instance_id: u32,
  direct_env: Option<sys::napi_env>,
  direct_memory: Option<DirectMemory>,
}

#[derive(Clone, Copy)]
struct DirectMemory {
  ptr: *mut u8,
  len: usize,
}

impl NodeWasmCaller {
  fn run(&self, op: JsOp) -> anyhow::Result<JsResult> {
    if let Some(env) = self.direct_env {
      execute_js_op(&self.host, env, op)
    } else {
      run_js(&self.host, op)
    }
  }
}

impl runtime::Caller<'_> for NodeWasmCaller {
  fn read_buf(&self, ptr: u32, buf: &mut [u8]) -> anyhow::Result<()> {
    if let Some(memory) = self.direct_memory {
      let start = ptr as usize;
      let end = start
        .checked_add(buf.len())
        .context("SWC Wasm read range overflowed")?;
      if end > memory.len {
        anyhow::bail!(
          "SWC Wasm read out of bounds: ptr={ptr}, len={}, memory={}",
          buf.len(),
          memory.len
        );
      }
      unsafe {
        std::ptr::copy_nonoverlapping(memory.ptr.add(start), buf.as_mut_ptr(), buf.len());
      }
      return Ok(());
    }

    match self.run(JsOp::Read {
      instance_id: self.instance_id,
      ptr,
      len: buf
        .len()
        .try_into()
        .context("SWC Wasm read length does not fit into u32")?,
    })? {
      JsResult::Bytes(bytes) => {
        if bytes.len() != buf.len() {
          anyhow::bail!(
            "SWC Wasm read returned {} bytes, expected {} bytes",
            bytes.len(),
            buf.len()
          );
        }
        buf.copy_from_slice(&bytes);
        Ok(())
      }
      _ => unreachable!("read must return bytes"),
    }
  }

  fn write_buf(&mut self, ptr: u32, buf: &[u8]) -> anyhow::Result<()> {
    if let Some(memory) = self.direct_memory {
      let start = ptr as usize;
      let end = start
        .checked_add(buf.len())
        .context("SWC Wasm write range overflowed")?;
      if end > memory.len {
        anyhow::bail!(
          "SWC Wasm write out of bounds: ptr={ptr}, len={}, memory={}",
          buf.len(),
          memory.len
        );
      }
      unsafe {
        std::ptr::copy_nonoverlapping(buf.as_ptr(), memory.ptr.add(start), buf.len());
      }
      return Ok(());
    }

    self.run(JsOp::Write {
      instance_id: self.instance_id,
      ptr,
      bytes: buf.to_vec(),
    })?;
    Ok(())
  }

  fn alloc(&mut self, size: u32) -> anyhow::Result<u32> {
    // `__alloc` may grow WebAssembly.Memory, which detaches the old
    // ArrayBuffer. Fall back to JS-side memory access after allocation.
    self.direct_memory = None;
    match self.run(JsOp::Alloc {
      instance_id: self.instance_id,
      size,
    })? {
      JsResult::U32(value) => Ok(value),
      _ => unreachable!("alloc must return u32"),
    }
  }

  fn free(&mut self, ptr: u32, size: u32) -> anyhow::Result<u32> {
    match self.run(JsOp::Free {
      instance_id: self.instance_id,
      ptr,
      size,
    })? {
      JsResult::U32(value) => Ok(value),
      _ => unreachable!("free must return u32"),
    }
  }
}

enum JsOp {
  Compile {
    module_id: u32,
    bytes: Vec<u8>,
  },
  Instantiate {
    instance_id: u32,
    module_id: u32,
    import_names: Vec<String>,
    envs_json: String,
  },
  Transform {
    instance_id: u32,
    program_ptr: u32,
    program_len: u32,
    unresolved_mark: u32,
    should_enable_comments_proxy: u32,
  },
  Alloc {
    instance_id: u32,
    size: u32,
  },
  Free {
    instance_id: u32,
    ptr: u32,
    size: u32,
  },
  Read {
    instance_id: u32,
    ptr: u32,
    len: u32,
  },
  Write {
    instance_id: u32,
    ptr: u32,
    bytes: Vec<u8>,
  },
  DropInstance {
    instance_id: u32,
  },
  DropModule {
    module_id: u32,
  },
}

enum JsResult {
  Unit,
  U32(u32),
  Bytes(Vec<u8>),
}

struct JsTask {
  host: Arc<NodeWasmHost>,
  op: JsOp,
  tx: mpsc::SyncSender<anyhow::Result<JsResult>>,
}

extern "C" fn node_wasm_task_callback(
  env: sys::napi_env,
  _js_callback: sys::napi_value,
  _context: *mut c_void,
  data: *mut c_void,
) {
  if data.is_null() {
    return;
  }

  let task: Box<JsTask> = unsafe { Box::from_raw(data.cast()) };
  let result = if env.is_null() {
    Err(anyhow!(
      "Node.js WebAssembly runtime is shutting down before the SWC Wasm task completed"
    ))
  } else {
    execute_js_op(&task.host, env, task.op)
  };
  let _ = task.tx.send(result);
}

fn run_js(host: &Arc<NodeWasmHost>, op: JsOp) -> anyhow::Result<JsResult> {
  if std::thread::current().id() == host.js_thread_id {
    return execute_js_op(host, host.env, op);
  }

  let (tx, rx) = mpsc::sync_channel(1);
  let task = Box::new(JsTask {
    host: Arc::clone(host),
    op,
    tx,
  });
  let raw_task = Box::into_raw(task);
  let status = unsafe {
    sys::napi_call_threadsafe_function(
      host.task_tsfn,
      raw_task.cast(),
      sys::ThreadsafeFunctionCallMode::blocking,
    )
  };
  if status != sys::Status::napi_ok {
    unsafe { drop(Box::from_raw(raw_task)) };
    anyhow::bail!(
      "Failed to schedule Node.js WebAssembly runtime task: {}",
      Status::from(status)
    );
  }

  rx.recv()
    .context("Node.js WebAssembly runtime task channel closed")?
}

fn execute_js_op(host: &NodeWasmHost, env: sys::napi_env, op: JsOp) -> anyhow::Result<JsResult> {
  let env = unsafe { Env::from_raw(env) };
  let helper = helper_object(host, &env)?;

  let result = match op {
    JsOp::Compile { module_id, bytes } => {
      let compile = helper
        .get_named_property::<Function<'_, FnArgs<(u32, Buffer)>, ()>>("compile")
        .map_err(napi_to_anyhow)?;
      compile
        .call(FnArgs::from((module_id, Buffer::from(bytes))))
        .map_err(napi_to_anyhow)?;
      JsResult::Unit
    }
    JsOp::Instantiate {
      instance_id,
      module_id,
      import_names,
      envs_json,
    } => {
      let instantiate = helper
        .get_named_property::<Function<'_, FnArgs<(u32, u32, Vec<String>, String)>, ()>>(
          "instantiate",
        )
        .map_err(napi_to_anyhow)?;
      instantiate
        .call(FnArgs::from((
          instance_id,
          module_id,
          import_names,
          envs_json,
        )))
        .map_err(napi_to_anyhow)?;
      JsResult::Unit
    }
    JsOp::Transform {
      instance_id,
      program_ptr,
      program_len,
      unresolved_mark,
      should_enable_comments_proxy,
    } => {
      let transform = helper
        .get_named_property::<Function<'_, FnArgs<(u32, u32, u32, u32, u32)>, u32>>("transform")
        .map_err(napi_to_anyhow)?;
      JsResult::U32(
        transform
          .call(FnArgs::from((
            instance_id,
            program_ptr,
            program_len,
            unresolved_mark,
            should_enable_comments_proxy,
          )))
          .map_err(napi_to_anyhow)?,
      )
    }
    JsOp::Alloc { instance_id, size } => {
      let alloc = helper
        .get_named_property::<Function<'_, FnArgs<(u32, u32)>, u32>>("alloc")
        .map_err(napi_to_anyhow)?;
      JsResult::U32(
        alloc
          .call(FnArgs::from((instance_id, size)))
          .map_err(napi_to_anyhow)?,
      )
    }
    JsOp::Free {
      instance_id,
      ptr,
      size,
    } => {
      let free = helper
        .get_named_property::<Function<'_, FnArgs<(u32, u32, u32)>, u32>>("free")
        .map_err(napi_to_anyhow)?;
      JsResult::U32(
        free
          .call(FnArgs::from((instance_id, ptr, size)))
          .map_err(napi_to_anyhow)?,
      )
    }
    JsOp::Read {
      instance_id,
      ptr,
      len,
    } => {
      let read = helper
        .get_named_property::<Function<'_, FnArgs<(u32, u32, u32)>, Buffer>>("read")
        .map_err(napi_to_anyhow)?;
      let bytes = read
        .call(FnArgs::from((instance_id, ptr, len)))
        .map(Vec::<u8>::from)
        .map_err(napi_to_anyhow)?;
      JsResult::Bytes(bytes)
    }
    JsOp::Write {
      instance_id,
      ptr,
      bytes,
    } => {
      let write = helper
        .get_named_property::<Function<'_, FnArgs<(u32, u32, Buffer)>, ()>>("write")
        .map_err(napi_to_anyhow)?;
      write
        .call(FnArgs::from((instance_id, ptr, Buffer::from(bytes))))
        .map_err(napi_to_anyhow)?;
      JsResult::Unit
    }
    JsOp::DropInstance { instance_id } => {
      let drop_instance = helper
        .get_named_property::<Function<'_, u32, ()>>("dropInstance")
        .map_err(napi_to_anyhow)?;
      drop_instance.call(instance_id).map_err(napi_to_anyhow)?;
      JsResult::Unit
    }
    JsOp::DropModule { module_id } => {
      let drop_module = helper
        .get_named_property::<Function<'_, u32, ()>>("dropModule")
        .map_err(napi_to_anyhow)?;
      drop_module.call(module_id).map_err(napi_to_anyhow)?;
      JsResult::Unit
    }
  };

  Ok(result)
}

fn helper_object<'env>(host: &NodeWasmHost, env: &'env Env) -> anyhow::Result<Object<'env>> {
  let mut value = ptr::null_mut();
  let status = unsafe { sys::napi_get_reference_value(env.raw(), host.helper_ref, &mut value) };
  if status != sys::Status::napi_ok {
    anyhow::bail!(
      "Failed to get Node.js WebAssembly runtime helper: {}",
      Status::from(status)
    );
  }
  unsafe { Object::from_napi_value(env.raw(), value) }.map_err(napi_to_anyhow)
}

fn napi_to_anyhow(error: napi::Error) -> anyhow::Error {
  anyhow!(error.to_string())
}

fn check_napi_status(status: sys::napi_status, message: &'static str) -> napi::Result<()> {
  if status == sys::Status::napi_ok {
    Ok(())
  } else {
    Err(napi::Error::new(Status::from(status), message))
  }
}
