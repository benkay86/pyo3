// Copyright (c) 2017-present PyO3 Project and Contributors
//
// based on Daniel Grunwald's https://github.com/dgrunwald/rust-cpython

use crate::callback::IntoPyCallbackOutput;
use crate::err::{PyErr, PyResult};
use crate::exceptions;
use crate::ffi;
use crate::instance::PyNativeType;
use crate::pyclass::PyClass;
use crate::type_object::PyTypeObject;
use crate::types::{PyAny, PyDict, PyList};
use crate::types::{PyCFunction, PyTuple};
use crate::{AsPyPointer, IntoPy, Py, PyObject, Python};
use std::ffi::{CStr, CString};
use std::str;

/// Represents a Python [`module`][1] object.
///
/// As with all other Python objects, modules are first class citizens.
/// This means they can be passed to or returned from functions,
/// created dynamically, assigned to variables and so forth.
///
/// [1]: https://docs.python.org/3/tutorial/modules.html
#[repr(transparent)]
pub struct PyModule(PyAny);

pyobject_native_type_core!(PyModule, ffi::PyModule_Type, #checkfunction=ffi::PyModule_Check);

impl PyModule {
    /// Creates a new module object with the `__name__` attribute set to `name`.
    ///
    /// # Examples
    ///
    /// ``` rust
    /// use pyo3::prelude::*;
    ///
    /// # fn main() -> PyResult<()>{
    /// Python::with_gil(|py| -> PyResult<()>{
    ///     let module = PyModule::new(py, "my_module")?;
    ///
    ///     assert_eq!(module.name()?, "my_module");
    ///     Ok(())
    /// })?;
    /// # Ok(())}
    ///  ```
    pub fn new<'p>(py: Python<'p>, name: &str) -> PyResult<&'p PyModule> {
        // Could use PyModule_NewObject, but it doesn't exist on PyPy.
        let name = CString::new(name)?;
        unsafe { py.from_owned_ptr_or_err(ffi::PyModule_New(name.as_ptr())) }
    }

    /// Imports the Python module with the specified name.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main(){
    /// use pyo3::prelude::*;
    ///
    /// Python::with_gil(|py| {
    ///     let module = PyModule::import(py, "antigravity").expect("No flying for you.");
    /// });
    /// # }
    ///  ```
    ///
    /// This is equivalent to the following Python expression:
    ///
    /// ```python
    /// import antigravity
    /// ```
    pub fn import<'p>(py: Python<'p>, name: &str) -> PyResult<&'p PyModule> {
        let name: PyObject = name.into_py(py);
        unsafe { py.from_owned_ptr_or_err(ffi::PyImport_Import(name.as_ptr())) }
    }

    /// Creates and loads a module named `module_name`,
    /// containing the Python code passed to `code`
    /// and pretending to live at `file_name`.
    ///
    /// <div class="information">
    ///     <div class="tooltip compile_fail" style="">&#x26a0; &#xfe0f;</div>
    /// </div><div class="example-wrap" style="display:inline-block"><pre class="compile_fail" style="white-space:normal;font:inherit;">
    //
    ///  <strong>Warning</strong>: This will compile and execute code. <strong>Never</strong> pass untrusted code to this function!
    ///
    /// </pre></div>
    ///
    /// # Errors
    ///
    /// Returns `PyErr` if:
    /// - `code` is not syntactically correct Python.
    /// - Any Python exceptions are raised while initializing the module.
    /// - Any of the arguments cannot be converted to [`CString`](std::ffi::CString)s.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use pyo3::prelude::*;
    ///
    /// # fn main() -> PyResult<()> {
    /// Python::with_gil(|py| -> PyResult<()> {
    ///    let module = PyModule::from_code(
    ///         py,
    ///         "print(__file__, __name__)",
    ///         "my_file",
    ///         "my_module"
    ///     )?;
    ///     Ok(())
    /// })?;
    /// # Ok(())}
    /// ```
    pub fn from_code<'p>(
        py: Python<'p>,
        code: &str,
        file_name: &str,
        module_name: &str,
    ) -> PyResult<&'p PyModule> {
        let data = CString::new(code)?;
        let filename = CString::new(file_name)?;
        let module = CString::new(module_name)?;

        unsafe {
            let cptr = ffi::Py_CompileString(data.as_ptr(), filename.as_ptr(), ffi::Py_file_input);
            if cptr.is_null() {
                return Err(PyErr::api_call_failed(py));
            }

            let mptr = ffi::PyImport_ExecCodeModuleEx(module.as_ptr(), cptr, filename.as_ptr());
            if mptr.is_null() {
                return Err(PyErr::api_call_failed(py));
            }

            <&PyModule as crate::FromPyObject>::extract(py.from_owned_ptr_or_err(mptr)?)
        }
    }

    /// Returns the module's `__dict__` attribute, which contains the module's symbol table.
    pub fn dict(&self) -> &PyDict {
        unsafe {
            // PyModule_GetDict returns borrowed ptr; must make owned for safety (see #890).
            let ptr = ffi::PyModule_GetDict(self.as_ptr());
            ffi::Py_INCREF(ptr);
            self.py().from_owned_ptr(ptr)
        }
    }

    /// Returns the index (the `__all__` attribute) of the module,
    /// creating one if needed.
    ///
    /// `__all__` declares the items that will be imported with `from my_module import *`.
    pub fn index(&self) -> PyResult<&PyList> {
        match self.getattr("__all__") {
            Ok(idx) => idx.downcast().map_err(PyErr::from),
            Err(err) => {
                if err.is_instance::<exceptions::PyAttributeError>(self.py()) {
                    let l = PyList::empty(self.py());
                    self.setattr("__all__", l).map_err(PyErr::from)?;
                    Ok(l)
                } else {
                    Err(err)
                }
            }
        }
    }

    /// Returns the name (the `__name__` attribute) of the module.
    ///
    /// May fail if the module does not have a `__name__` attribute.
    pub fn name(&self) -> PyResult<&str> {
        let ptr = unsafe { ffi::PyModule_GetName(self.as_ptr()) };
        if ptr.is_null() {
            Err(PyErr::api_call_failed(self.py()))
        } else {
            let name = unsafe { CStr::from_ptr(ptr) }
                .to_str()
                .expect("PyModule_GetName expected to return utf8");
            Ok(name)
        }
    }

    /// Returns the filename (the `__file__` attribute) of the module.
    ///
    /// May fail if the module does not have a `__file__` attribute.
    #[cfg(not(all(windows, PyPy)))]
    #[cfg_attr(docsrs, doc(cfg(not(all(windows, PyPy)))))]
    pub fn filename(&self) -> PyResult<&str> {
        use crate::types::PyString;
        unsafe {
            self.py()
                .from_owned_ptr_or_err::<PyString>(ffi::PyModule_GetFilenameObject(self.as_ptr()))?
                .to_str()
        }
    }

    /// Adds an attribute to the module.
    ///
    /// For adding classes, functions or modules, prefer to use [`PyModule::add_class`],
    /// [`PyModule::add_function`] or [`PyModule::add_submodule`] instead, respectively.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use pyo3::prelude::*;
    ///
    /// #[pymodule]
    /// fn my_module(_py: Python, module: &PyModule) -> PyResult<()> {
    ///     module.add("c", 299_792_458)?;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// Python code can then do the following:
    ///
    /// ```python
    /// from my_module import c
    ///
    /// print("c is", c)
    /// ```
    ///
    /// This will result in the following output:
    ///
    /// ```text
    /// c is 299792458
    /// ```
    pub fn add<V>(&self, name: &str, value: V) -> PyResult<()>
    where
        V: IntoPy<PyObject>,
    {
        self.index()?
            .append(name)
            .expect("could not append __name__ to __all__");
        self.setattr(name, value.into_py(self.py()))
    }

    /// Adds a new class to the module.
    ///
    /// Notice that this method does not take an argument.
    /// Instead, this method is *generic*, and requires us to use the
    /// "turbofish" syntax to specify the class we want to add.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use pyo3::prelude::*;
    ///
    /// #[pyclass]
    /// struct Foo { /* fields omitted */ }
    ///
    /// #[pymodule]
    /// fn my_module(_py: Python, module: &PyModule) -> PyResult<()> {
    ///     module.add_class::<Foo>()?;
    ///     Ok(())
    /// }
    ///  ```
    ///
    /// Python code can see this class as such:
    ///
    /// ```python
    /// from my_module import Foo
    ///
    /// print("Foo is", Foo)
    /// ```
    ///
    /// This will result in the following output:
    ///
    /// ```text
    /// Foo is <class 'builtins.Foo'>
    /// ```
    ///
    /// Note that as we haven't defined a [constructor][1], Python code can't actually
    /// make an *instance* of `Foo` (or *get* one for that matter, as we haven't exported
    /// anything that can return instances of `Foo`).
    ///
    /// [1]: https://pyo3.rs/latest/class.html#constructor
    pub fn add_class<T>(&self) -> PyResult<()>
    where
        T: PyClass,
    {
        self.add(T::NAME, <T as PyTypeObject>::type_object(self.py()))
    }

    /// Adds a function or a (sub)module to a module, using the functions name as name.
    ///
    /// Prefer to use [`PyModule::add_function`] and/or [`PyModule::add_submodule`] instead.
    pub fn add_wrapped<'a, T>(&'a self, wrapper: &impl Fn(Python<'a>) -> T) -> PyResult<()>
    where
        T: IntoPyCallbackOutput<PyObject>,
    {
        let py = self.py();
        let function = wrapper(py).convert(py)?;
        let name = function.getattr(py, "__name__")?;
        let name = name.extract(py)?;
        self.add(name, function)
    }

    /// Adds a submodule to a module.
    ///
    /// This is especially useful for creating module hierarchies.
    ///
    /// Note that this doesn't define a *package*, so this won't allow Python code
    /// to directly import submodules by using
    /// <span style="white-space: pre">`from my_module import submodule`</span>.
    /// For more information, see [#759][1] and [#1517][2].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use pyo3::prelude::*;
    ///
    /// #[pymodule]
    /// fn my_module(py: Python, module: &PyModule) -> PyResult<()> {
    ///     let submodule = PyModule::new(py, "submodule")?;
    ///     submodule.add("super_useful_constant", "important")?;
    ///
    ///     module.add_submodule(submodule)?;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// Python code can then do the following:
    ///
    /// ```python
    /// import my_module
    ///
    /// print("super_useful_constant is", my_module.submodule.super_useful_constant)
    /// ```
    ///
    /// This will result in the following output:
    ///
    /// ```text
    /// super_useful_constant is important
    /// ```
    ///
    /// [1]: https://github.com/PyO3/pyo3/issues/759
    /// [2]: https://github.com/PyO3/pyo3/issues/1517#issuecomment-808664021
    pub fn add_submodule(&self, module: &PyModule) -> PyResult<()> {
        let name = module.name()?;
        self.add(name, module)
    }

    /// Add a function to a module.
    ///
    /// Note that this also requires the [`wrap_pyfunction!`][2] macro
    /// to wrap a function annotated with [`#[pyfunction]`][1].
    ///
    /// ```rust
    /// use pyo3::prelude::*;
    ///
    /// #[pyfunction]
    /// fn say_hello() {
    ///     println!("Hello world!")
    /// }
    /// #[pymodule]
    /// fn my_module(_py: Python, module: &PyModule) -> PyResult<()> {
    ///     module.add_function(wrap_pyfunction!(say_hello, module)?)
    /// }
    /// ```
    ///
    /// Python code can then do the following:
    ///
    /// ```python
    /// from my_module import say_hello
    ///
    /// say_hello()
    /// ```
    ///
    /// This will result in the following output:
    ///
    /// ```text
    /// Hello world!
    /// ```
    ///
    /// [1]: crate::prelude::pyfunction
    /// [2]: crate::wrap_pyfunction
    pub fn add_function<'a>(&'a self, fun: &'a PyCFunction) -> PyResult<()> {
        let name = fun.getattr("__name__")?.extract()?;
        self.add(name, fun)
    }

    /// Calls a function in the module.
    ///
    /// This is equivalent to the Python expression `module.name(*args, **kwargs)`.
    #[deprecated(
        since = "0.14.0",
        note = "use getattr(name)?.call(args, kwargs) instead"
    )]
    pub fn call(
        &self,
        name: &str,
        args: impl IntoPy<Py<PyTuple>>,
        kwargs: Option<&PyDict>,
    ) -> PyResult<&PyAny> {
        self.getattr(name)?.call(args, kwargs)
    }

    /// Calls a function in the module with only positional arguments.
    ///
    /// This is equivalent to the Python expression `module.name(*args)`.
    #[deprecated(since = "0.14.0", note = "use getattr(name)?.call1(args) instead")]
    pub fn call1(&self, name: &str, args: impl IntoPy<Py<PyTuple>>) -> PyResult<&PyAny> {
        self.getattr(name)?.call1(args)
    }

    /// Calls a function in the module without arguments.
    ///
    /// This is equivalent to the Python expression `module.name()`.
    #[deprecated(since = "0.14.0", note = "use getattr(name)?.call0() instead")]
    pub fn call0(&self, name: &str) -> PyResult<&PyAny> {
        self.getattr(name)?.call0()
    }

    /// Gets a member from the module.
    ///
    /// This is equivalent to the Python expression `module.name`.
    #[deprecated(since = "0.14.0", note = "use getattr(name) instead")]
    pub fn get(&self, name: &str) -> PyResult<&PyAny> {
        self.getattr(name)
    }
}

#[cfg(test)]
mod tests {
    use crate::{types::PyModule, Python};

    #[test]
    fn module_import_and_name() {
        Python::with_gil(|py| {
            let builtins = PyModule::import(py, "builtins").unwrap();
            assert_eq!(builtins.name().unwrap(), "builtins");
        })
    }
}
