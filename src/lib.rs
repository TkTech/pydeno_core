use {
    deno_core::{
        op_sync, JsRuntime, OpState, RuntimeOptions, ZeroCopyBuf
    },
    pyo3::prelude::*,
    pythonize::{depythonize, pythonize}
};

/// Determine if the passed callable object is a coroutine function.
/// Internally, this is calling Python's asyncio.iscoroutinefunction().
// Do *not* use inspect.iscoroutinefunction.
pub fn is_async(callback: &PyObject) -> bool {
    Python::with_gil(|py| {
        // Can this ever fail?
        let inspect = PyModule::import(py, "asyncio").unwrap();

        match inspect.call1("iscoroutinefunction", (callback.to_object(py),)) {
            Ok(result) => match result.extract() {
                Ok(result) => result,
                Err(_) => false,
            },
            Err(_) => false,
        }
    })
}



/// A wrapper around the deno_core JsRuntime.
///
/// Types deep within the JsRuntime, such as v8::isolate, are !Send and are
/// unsafe to migrate. deno_core always enters and exits an isolate, so we
/// have no way to migrate it between threads. Thus, Runtime is cannot be
/// shared between threads in Python.
#[pyclass(unsendable, module = "deno_core")]
struct Runtime {
    runtime: JsRuntime,
}

#[pymethods]
impl Runtime {
    #[new]
    fn new() -> PyResult<Self> {
        let runtime = JsRuntime::new(RuntimeOptions {
            ..Default::default()
        });

        Ok(Runtime { runtime })
    }

    /// Execute some JavaScript within the Sandbox, preserving local
    /// state between calls.
    pub fn execute(&mut self, filename: &str, source: &str) -> PyResult<()> {
        self.runtime.execute(filename, source).unwrap();
        Ok(())
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
    pub fn on(&mut self, op_name: &str, callback: PyObject) -> PyResult<()> {
        let runner =
            move |_state: &mut OpState,
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
}

#[pymodule]
fn deno_core(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<Runtime>()?;

    Ok(())
}
