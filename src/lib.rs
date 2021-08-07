use {
    deno_core::{
        op_async, op_sync, JsRuntime, OpState, RuntimeOptions, Snapshot,
        ZeroCopyBuf, v8
    },
    futures::channel::oneshot,
    pyo3::{exceptions::*, prelude::*},
    pythonize::{depythonize, pythonize},
    std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

/// Determine if the passed callable object is a coroutine function.
pub fn is_async(callback: &PyObject) -> bool {
    Python::with_gil(|py| {
        let inspect = PyModule::import(py, "asyncio").unwrap();
        let method = inspect.getattr("iscoroutinefunction").unwrap();

        match method.call1((callback.to_object(py),)) {
            Ok(result) => match result.extract() {
                Ok(result) => result,
                Err(_) => false,
            },
            Err(_) => false,
        }
    })
}

#[pyclass]
struct PyFuture {
    awaitable: PyObject,
    tx: Option<oneshot::Sender<PyResult<PyObject>>>,
}

#[pymethods]
impl PyFuture {
    #[call]
    #[args(task)]
    pub fn __call__(&mut self, self_: pyo3::Py<Self>) -> PyResult<()> {
        Python::with_gil(|py| {
            let asyncio = py.import("asyncio")?;
            let ensure = asyncio.getattr("ensure_future")?;
            let task = ensure.call1((self.awaitable.as_ref(py),))?;

            // let task = asyncio.call1("create_task", (self.awaitable.as_ref(py),)).unwrap();

            let callback = self_.getattr(py, "callback")?;
            task.call_method1("add_done_callback", (callback,))?;

            Ok(())
        })
    }

    pub fn callback(&mut self, task: &PyAny) -> PyResult<()> {
        let result = match task.call_method0("result") {
            Ok(v) => Ok(v.into()),
            Err(e) => Err(e),
        };

        if let Some(tx) = self.tx.take() {
            if tx.send(result).is_err() {
                // Do what?
            }
        }
        Ok(())
    }
}

/// A wrapper around the deno_core JsRuntime.
///
/// Types deep within the JsRuntime, such as v8::isolate, are !Send and are
/// unsafe to migrate. deno_core always enters and exits an isolate, so we
/// have no way to migrate it between threads. Thus, Runtime is cannot be
/// shared between threads in Python.
#[pyclass(unsendable, module = "deno_core")]
#[pyo3(text_signature = "(will_snapshot, startup_snapshot)")]
struct Runtime {
    runtime: JsRuntime,
    waker: Waker,
}

type WakerData = *const ();
unsafe fn clone(_: WakerData) -> RawWaker {
    RawWaker::new(std::ptr::null(), &WAKER_VTABLE)
}
unsafe fn wake(_: WakerData) {
    println!("Wake was called!");
}
unsafe fn wake_by_ref(_: WakerData) {}
unsafe fn drop(_: WakerData) {}

static WAKER_VTABLE: RawWakerVTable =
    RawWakerVTable::new(clone, wake, wake_by_ref, drop);

#[pymethods]
impl Runtime {
    #[new]
    #[args(
        "*",
        will_snapshot = "false",
        startup_snapshot = "None",
        initial_heap = "None",
        maximum_heap = "None"
    )]
    fn new(
        will_snapshot: bool,
        startup_snapshot: Option<Vec<u8>>,
        initial_heap: Option<usize>,
        maximum_heap: Option<usize>,
    ) -> PyResult<Self> {
        let mut params = v8::Isolate::create_params();

        // Setting the heap limits only makes sense with both the initial
        // and the maximum.
        if let (Some(initial), Some(maximum)) = (initial_heap, maximum_heap) {
            params = params.heap_limits(initial, maximum);
        }

        // A special, very niche waker. We know the deno_core JsRuntime's
        // event loop only ever uses Waker::wake(), so that's all we even
        // bother to implement.
        let waker = unsafe {
            Waker::from_raw(RawWaker::new(std::ptr::null(), &WAKER_VTABLE))
        };

        let snapshot = match startup_snapshot {
            Some(ss) => Some(Snapshot::Boxed(ss.into_boxed_slice())),
            None => None,
        };

        Ok(Runtime {
            runtime: JsRuntime::new(RuntimeOptions {
                will_snapshot,
                startup_snapshot: snapshot,
                create_params: Some(params),
                ..Default::default()
            }),
            waker: waker,
        })
    }

    /// Execute some JavaScript within the Sandbox, preserving local
    /// state between calls.
    #[pyo3(text_signature = "($self, filename, source)")]
    pub fn execute_script(
        &mut self,
        filename: &str,
        source: &str,
    ) -> PyResult<()> {
        self.runtime.execute_script(filename, source).unwrap();
        Ok(())
    }

    /// Run the event loop just once. Returns True if there is more work to
    /// be done, False otherwise.
    #[pyo3(text_signature = "($self)")]
    pub fn poll_once(&mut self) -> PyResult<PyObject> {
        let mut context = Context::from_waker(&self.waker);

        match self.runtime.poll_event_loop(&mut context, false) {
            Poll::Ready(v) => match v {
                Ok(_) => Python::with_gil(|py| {
                    // Is there truly no macro for True & False? This feels
                    // very verbose.
                    Ok(pyo3::types::PyBool::new(py, false).to_object(py))
                }),
                // TODO: This should be a much more specific error.
                Err(e) => Err(PyRuntimeError::new_err(format!("{:?}", e))),
            },
            Poll::Pending => Python::with_gil(|py| {
                Ok(pyo3::types::PyBool::new(py, true).to_object(py))
            }),
        }
    }

    /// Register a Python callable that can be triggered by
    /// Deno.core.opSync.
    ///
    /// This works by bi-directional JSON encoding of arguments and return
    /// values. Try to do as much work as possible within a single call to
    /// avoid the high overhead of this method of isolation.
    //
    // This isn't even remotely efficient, but it is quick and simple. We
    // can do better, especially if we're willing to modify deno_core to
    // skip over serde_v8.
    #[pyo3(text_signature = "($self, op_name, callback)")]
    pub fn on(&mut self, op_name: &str, callback: PyObject) -> PyResult<()> {
        let runner = move |_state: &mut OpState,
                           v: serde_json::Value,
                           _: Option<ZeroCopyBuf>| {
            Python::with_gil(|py| {
                let args = pythonize(py, &v).unwrap();
                let result = callback.call1(py, (args,)).unwrap();
                let return_value: serde_json::Value =
                    depythonize(result.as_ref(py)).unwrap();
                Ok(return_value)
            })
        };

        self.runtime.register_op(op_name, op_sync(runner));
        self.runtime.sync_ops_cache();

        Ok(())
    }

    #[pyo3(text_signature = "($self, op_name, callback)")]
    pub fn on_async(
        &mut self,
        op_name: &str,
        callback: PyObject,
    ) -> PyResult<()> {
        let runner = move |_state,
                           v: serde_json::Value,
                           _: Option<ZeroCopyBuf>| {
            let (tx, rx) = oneshot::channel();

            Python::with_gil(|py| {
                let asyncio = py.import("asyncio").unwrap();
                let ev_loop = asyncio.call_method0("get_event_loop").unwrap();
                let call_soon =
                    ev_loop.getattr("call_soon_threadsafe").unwrap();

                let args = pythonize(py, &v).unwrap();

                // Turn the function into a Coroutine by calling it with its
                // args.
                let coro = callback.call1(py, (args,)).unwrap();

                call_soon
                    .call1((PyFuture {
                        awaitable: coro.into(),
                        tx: Some(tx),
                    },))
                    .unwrap();
            });

            async move {
                match rx.await {
                    Ok(v) => match v {
                        Ok(vv) => Python::with_gil(|py| {
                            let return_value: serde_json::Value =
                                depythonize(vv.as_ref(py)).unwrap();
                            Ok(return_value)
                        }),
                        Err(e) => {
                            Err(deno_core::error::generic_error("Unknown"))
                        }
                    },
                    Err(e) => {
                        // Raised when the oneshot::channel has been cancelled due to the sender
                        // dropping.
                        Err(deno_core::error::generic_error("CancelledError"))
                    }
                }
            }
        };

        self.runtime.register_op(op_name, op_async(runner));
        self.runtime.sync_ops_cache();

        Ok(())
    }

    /// Generate a binary snapshot of the runtime's state, which can be passed
    /// into new runtimes. The runtime must have been started with
    /// ``will_snapshot=True``.
    #[pyo3(text_signature = "($self)")]
    pub fn snapshot(&mut self, py: Python) -> PyObject {
        let snapshot = self.runtime.snapshot();
        let slice: &[u8] = &*snapshot;
        pyo3::types::PyBytes::new(py, &slice).into()
    }

    /// Sets a callback to be called when the runtime approachs the heap
    /// limits. The value returned by the callback will be used as the new
    /// heap limit.
    ///
    /// If multiple callbacks are added, only the most recent is used.
    #[pyo3(text_signature = "($self, callback)")]
    pub fn near_heap_limit(&mut self, callback: PyObject) -> PyResult<()> {
        self.runtime.add_near_heap_limit_callback(
            move |current, initial| -> usize {
                Python::with_gil(|py| {
                    let new_limit =
                        callback.call1(py, (current, initial)).unwrap();

                    new_limit.extract(py).unwrap()
                })
            },
        );
        Ok(())
    }

    /// Removes any existing `near_heap_limit` callback, setting the new limit
    /// to `limit`. A limit of ``0`` will disable the limit, and a limit that
    /// is to small for the current heap will be resized to the minimum
    /// required.
    #[pyo3(text_signature = "($self, new_limit)")]
    pub fn clear_near_heap_limit(&mut self, new_limit: usize) -> PyResult<()> {
        self.runtime.remove_near_heap_limit_callback(new_limit);
        Ok(())
    }

    pub fn terminate_execution(&mut self) -> PyResult<bool> {
        Ok(self.runtime.v8_isolate().terminate_execution())
    }

    pub fn is_execution_terminating(&mut self) -> PyResult<bool> {
        Ok(self.runtime.v8_isolate().is_execution_terminating())
    }
}

#[pymodule]
fn deno_core(py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<Runtime>()?;

    Ok(())
}
