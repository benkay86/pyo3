# Python Classes

PyO3 exposes a group of attributes powered by Rust's proc macro system for defining Python classes as Rust structs.

The main attribute is `#[pyclass]`, which is placed upon a Rust `struct` to generate a Python type for it. A struct will usually also have *one* `#[pymethods]`-annotated `impl` block for the struct, which is used to define Python methods and constants for the generated Python type. (If the [`multiple-pymethods`] feature is enabled each `#[pyclass]` is allowed to have multiple `#[pymethods]` blocks.) Finally, there may be multiple `#[pyproto]` trait implementations for the struct, which are used to define certain python magic methods such as `__str__`.

This chapter will discuss the functionality and configuration these attributes offer. Below is a list of links to the relevant section of this chapter for each:

- [`#[pyclass]`](#defining-a-new-class)
  - [`#[pyo3(get, set)]`](#object-properties-using-pyo3get-set)
- [`#[pymethods]`](#instance-methods)
  - [`#[new]`](#constructor)
  - [`#[getter]`](#object-properties-using-getter-and-setter)
  - [`#[setter]`](#object-properties-using-getter-and-setter)
  - [`#[staticmethod]`](#static-methods)
  - [`#[classmethod]`](#class-methods)
  - [`#[call]`](#callable-objects)
  - [`#[classattr]`](#class-attributes)
  - [`#[args]`](#method-arguments)
- [`#[pyproto]`](class/protocols.html)

## Defining a new class

To define a custom Python class, a Rust struct needs to be annotated with the
`#[pyclass]` attribute.

```rust
# use pyo3::prelude::*;
#[pyclass]
struct MyClass {
    num: i32,
    debug: bool,
}
```

Because Python objects are freely shared between threads by the Python interpreter, all structs annotated with `#[pyclass]` must implement `Send` (unless annotated with [`#[pyclass(unsendable)]`](#customizing-the-class)).

The above example generates implementations for [`PyTypeInfo`], [`PyTypeObject`], and [`PyClass`] for `MyClass`. To see these generated implementations, refer to the [implementation details](#implementation-details) at the end of this chapter.

## Adding the class to a module

Custom Python classes can then be added to a module using `add_class()`.

```rust
# use pyo3::prelude::*;
# #[pyclass]
# struct MyClass {
#    num: i32,
#    debug: bool,
# }
#[pymodule]
fn mymodule(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<MyClass>()?;
    Ok(())
}
```

## PyCell and interior mutability

You sometimes need to convert your `pyclass` into a Python object and access it
from Rust code (e.g., for testing it).
[`PyCell`] is the primary interface for that.

`PyCell<T: PyClass>` is always allocated in the Python heap, so Rust doesn't have ownership of it.
In other words, Rust code can only extract a `&PyCell<T>`, not a `PyCell<T>`.

Thus, to mutate data behind `&PyCell` safely, PyO3 employs the
[Interior Mutability Pattern](https://doc.rust-lang.org/book/ch15-05-interior-mutability.html)
like [`RefCell`].

Users who are familiar with `RefCell` can use `PyCell` just like `RefCell`.

For users who are not very familiar with `RefCell`, here is a reminder of Rust's rules of borrowing:
- At any given time, you can have either (but not both of) one mutable reference or any number of immutable references.
- References must always be valid.

`PyCell`, like `RefCell`, ensures these borrowing rules by tracking references at runtime.

```rust
# use pyo3::prelude::*;
# use pyo3::types::PyDict;
#[pyclass]
struct MyClass {
    #[pyo3(get)]
    num: i32,
    debug: bool,
}
Python::with_gil(|py| {
    let obj = PyCell::new(py, MyClass { num: 3, debug: true }).unwrap();
    {
        let obj_ref = obj.borrow(); // Get PyRef
        assert_eq!(obj_ref.num, 3);
        // You cannot get PyRefMut unless all PyRefs are dropped
        assert!(obj.try_borrow_mut().is_err());
    }
    {
        let mut obj_mut = obj.borrow_mut(); // Get PyRefMut
        obj_mut.num = 5;
        // You cannot get any other refs until the PyRefMut is dropped
        assert!(obj.try_borrow().is_err());
        assert!(obj.try_borrow_mut().is_err());
    }

    // You can convert `&PyCell` to a Python object
    pyo3::py_run!(py, obj, "assert obj.num == 5");
});
```

`&PyCell<T>` is bounded by the same lifetime as a [`GILGuard`].
To make the object longer lived (for example, to store it in a struct on the
Rust side), you can use `Py<T>`, which stores an object longer than the GIL
lifetime, and therefore needs a `Python<'_>` token to access.

```rust
# use pyo3::prelude::*;
#[pyclass]
struct MyClass {
    num: i32,
}
fn return_myclass() -> Py<MyClass> {
    Python::with_gil(|py| Py::new(py, MyClass { num: 1 }).unwrap())
}
let obj = return_myclass();
Python::with_gil(|py|{
    let cell = obj.as_ref(py); // Py<MyClass>::as_ref returns &PyCell<MyClass>
    let obj_ref = cell.borrow(); // Get PyRef<T>
    assert_eq!(obj_ref.num, 1);
});
```

## Customizing the class

The `#[pyclass]` macro accepts the following parameters:

* `name="XXX"` - Set the class name shown in Python code. By default, the struct name is used as the class name.
* `freelist=XXX` - The `freelist` parameter adds support of free allocation list to custom class.
The performance improvement applies to types that are often created and deleted in a row,
so that they can benefit from a freelist. `XXX` is a number of items for the free list.
* `gc` - Classes with the `gc` parameter participate in Python garbage collection.
If a custom class contains references to other Python objects that can be collected, the [`PyGCProtocol`]({{#PYO3_DOCS_URL}}/pyo3/class/gc/trait.PyGCProtocol.html) trait has to be implemented.
* `weakref` - Adds support for Python weak references.
* `extends=BaseType` - Use a custom base class. The base `BaseType` must implement `PyTypeInfo`.
* `subclass` - Allows Python classes to inherit from this class.
* `dict` - Adds `__dict__` support, so that the instances of this type have a dictionary containing arbitrary instance variables.
* `unsendable` - Making it safe to expose `!Send` structs to Python, where all object can be accessed
   by multiple threads. A class marked with `unsendable` panics when accessed by another thread.
* `module="XXX"` - Set the name of the module the class will be shown as defined in. If not given, the class
  will be a virtual member of the `builtins` module.

## Constructor

By default it is not possible to create an instance of a custom class from Python code.
To declare a constructor, you need to define a method and annotate it with the `#[new]`
attribute. Only Python's `__new__` method can be specified, `__init__` is not available.

```rust
# use pyo3::prelude::*;
#[pyclass]
struct MyClass {
    num: i32,
}

#[pymethods]
impl MyClass {
    #[new]
    fn new(num: i32) -> Self {
        MyClass { num }
    }
}
```

Alternatively, if your `new` method may fail you can return `PyResult<Self>`.
```rust
# use pyo3::prelude::*;
#[pyclass]
struct MyClass {
    num: i32,
}

#[pymethods]
impl MyClass {
    #[new]
    fn new(num: i32) -> PyResult<Self> {
        Ok(MyClass { num })
    }
}
```

If no method marked with `#[new]` is declared, object instances can only be
created from Rust, but not from Python.

For arguments, see the `Method arguments` section below.

### Return type

Generally, `#[new]` method have to return `T: Into<PyClassInitializer<Self>>` or
`PyResult<T> where T: Into<PyClassInitializer<Self>>`.

For constructors that may fail, you should wrap the return type in a PyResult as well.
Consult the table below to determine which type your constructor should return:

|                             | **Cannot fail**           | **May fail**                      |
|-----------------------------|---------------------------|-----------------------------------|
|**No inheritance**           | `T`                       | `PyResult<T>`                     |
|**Inheritance(T Inherits U)**| `(T, U)`                  | `PyResult<(T, U)>`                |
|**Inheritance(General Case)**| [`PyClassInitializer<T>`] | `PyResult<PyClassInitializer<T>>` |

## Inheritance

By default, `PyAny` is used as the base class. To override this default,
use the `extends` parameter for `pyclass` with the full path to the base class.

For convenience, `(T, U)` implements `Into<PyClassInitializer<T>>` where `U` is the
baseclass of `T`.
But for more deeply nested inheritance, you have to return `PyClassInitializer<T>`
explicitly.

To get a parent class from a child, use [`PyRef`] instead of `&self` for methods,
or [`PyRefMut`] instead of `&mut self`.
Then you can access a parent class by `self_.as_ref()` as `&Self::BaseClass`,
or by `self_.into_super()` as `PyRef<Self::BaseClass>`.

```rust
# use pyo3::prelude::*;

#[pyclass(subclass)]
struct BaseClass {
    val1: usize,
}

#[pymethods]
impl BaseClass {
    #[new]
    fn new() -> Self {
        BaseClass { val1: 10 }
    }

    pub fn method(&self) -> PyResult<usize> {
        Ok(self.val1)
    }
}

#[pyclass(extends=BaseClass, subclass)]
struct SubClass {
    val2: usize,
}

#[pymethods]
impl SubClass {
    #[new]
    fn new() -> (Self, BaseClass) {
        (SubClass { val2: 15 }, BaseClass::new())
    }

    fn method2(self_: PyRef<Self>) -> PyResult<usize> {
        let super_ = self_.as_ref();  // Get &BaseClass
        super_.method().map(|x| x * self_.val2)
    }
}

#[pyclass(extends=SubClass)]
struct SubSubClass {
    val3: usize,
}

#[pymethods]
impl SubSubClass {
    #[new]
    fn new() -> PyClassInitializer<Self> {
        PyClassInitializer::from(SubClass::new())
            .add_subclass(SubSubClass{val3: 20})
    }

    fn method3(self_: PyRef<Self>) -> PyResult<usize> {
        let v = self_.val3;
        let super_ = self_.into_super();  // Get PyRef<SubClass>
        SubClass::method2(super_).map(|x| x * v)
    }
}
# Python::with_gil(|py| {
#     let subsub = pyo3::PyCell::new(py, SubSubClass::new()).unwrap();
#     pyo3::py_run!(py, subsub, "assert subsub.method3() == 3000")
# });
```

You can also inherit native types such as `PyDict`, if they implement
[`PySizedLayout`]({{#PYO3_DOCS_URL}}/pyo3/type_object/trait.PySizedLayout.html). However, this is not supported when building for the Python limited API (aka the `abi3` feature of PyO3).

However, because of some technical problems, we don't currently provide safe upcasting methods for types
that inherit native types. Even in such cases, you can unsafely get a base class by raw pointer conversion.

```rust
# #[cfg(not(Py_LIMITED_API))] {
# use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3::{AsPyPointer, PyNativeType};
use std::collections::HashMap;

#[pyclass(extends=PyDict)]
#[derive(Default)]
struct DictWithCounter {
    counter: HashMap<String, usize>,
}

#[pymethods]
impl DictWithCounter {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn set(mut self_: PyRefMut<Self>, key: String, value: &PyAny) -> PyResult<()> {
        self_.counter.entry(key.clone()).or_insert(0);
        let py = self_.py();
        let dict: &PyDict = unsafe { py.from_borrowed_ptr_or_err(self_.as_ptr())? };
        dict.set_item(key, value)
    }
}
# Python::with_gil(|py| {
#     let cnt = pyo3::PyCell::new(py, DictWithCounter::new()).unwrap();
#     pyo3::py_run!(py, cnt, "cnt.set('abc', 10); assert cnt['abc'] == 10")
# });
# }
```

If `SubClass` does not provide a baseclass initialization, the compilation fails.
```compile_fail
# use pyo3::prelude::*;

#[pyclass]
struct BaseClass {
    val1: usize,
}

#[pyclass(extends=BaseClass)]
struct SubClass {
    val2: usize,
}

#[pymethods]
impl SubClass {
    #[new]
    fn new() -> Self {
        SubClass { val2: 15 }
    }
}
```

## Object properties

PyO3 supports two ways to add properties to your `#[pyclass]`:
- For simple fields with no side effects, a `#[pyo3(get, set)]` attribute can be added directly to the field definition in the `#[pyclass]`.
- For properties which require computation you can define `#[getter]` and `#[setter]` functions in the [`#[pymethods]`](#instance-methods) block.

We'll cover each of these in the following sections.

### Object properties using `#[pyo3(get, set)]`

For simple cases where a member variable is just read and written with no side effects, you can declare getters and setters in your `#[pyclass]` field definition using the `pyo3` attribute, like in the example below:

```rust
# use pyo3::prelude::*;
#[pyclass]
struct MyClass {
    #[pyo3(get, set)]
    num: i32
}
```

The above would make the `num` field available for reading and writing as a `self.num` Python property. To expose the property with a different name to the field, specify this alongside the rest of the options, e.g. `#[pyo3(get, set, name = "custom_name")]`.

Properties can be readonly or writeonly by using just `#[pyo3(get)]` or `#[pyo3(set)]` respectively.

To use these annotations, your field type must implement some conversion traits:
- For `get` the field type must implement both `IntoPy<PyObject>` and `Clone`.
- For `set` the field type must implement `FromPyObject`.

### Object properties using `#[getter]` and `#[setter]`

For cases which don't satisfy the `#[pyo3(get, set)]` trait requirements, or need side effects, descriptor methods can be defined in a `#[pymethods]` `impl` block.

This is done using the `#[getter]` and `#[setter]` attributes, like in the example below:

```rust
# use pyo3::prelude::*;
#[pyclass]
struct MyClass {
    num: i32,
}

#[pymethods]
impl MyClass {
    #[getter]
    fn num(&self) -> PyResult<i32> {
        Ok(self.num)
    }
}
```

A getter or setter's function name is used as the property name by default. There are several
ways how to override the name.

If a function name starts with `get_` or `set_` for getter or setter respectively,
the descriptor name becomes the function name with this prefix removed. This is also useful in case of
Rust keywords like `type`
([raw identifiers](https://doc.rust-lang.org/edition-guide/rust-2018/module-system/raw-identifiers.html)
can be used since Rust 2018).

```rust
# use pyo3::prelude::*;
# #[pyclass]
# struct MyClass {
#     num: i32,
# }
#[pymethods]
impl MyClass {
    #[getter]
    fn get_num(&self) -> PyResult<i32> {
        Ok(self.num)
    }

    #[setter]
    fn set_num(&mut self, value: i32) -> PyResult<()> {
        self.num = value;
        Ok(())
    }
}
```

In this case, a property `num` is defined and available from Python code as `self.num`.

Both the `#[getter]` and `#[setter]` attributes accept one parameter.
If this parameter is specified, it is used as the property name, i.e.

```rust
# use pyo3::prelude::*;
# #[pyclass]
# struct MyClass {
#    num: i32,
# }
#[pymethods]
impl MyClass {
    #[getter(number)]
    fn num(&self) -> PyResult<i32> {
        Ok(self.num)
    }

    #[setter(number)]
    fn set_num(&mut self, value: i32) -> PyResult<()> {
        self.num = value;
        Ok(())
    }
}
```

In this case, the property `number` is defined and available from Python code as `self.number`.

Attributes defined by `#[setter]` or `#[pyo3(set)]` will always raise `AttributeError` on `del`
operations. Support for defining custom `del` behavior is tracked in
[#1778](https://github.com/PyO3/pyo3/issues/1778).

## Instance methods

To define a Python compatible method, an `impl` block for your struct has to be annotated with the
`#[pymethods]` attribute. PyO3 generates Python compatible wrappers for all functions in this
block with some variations, like descriptors, class method static methods, etc.

Since Rust allows any number of `impl` blocks, you can easily split methods
between those accessible to Python (and Rust) and those accessible only to Rust. However to have multiple
`#[pymethods]`-annotated `impl` blocks for the same struct you must enable the [`multiple-pymethods`] feature of PyO3.

```rust
# use pyo3::prelude::*;
# #[pyclass]
# struct MyClass {
#     num: i32,
# }
#[pymethods]
impl MyClass {
    fn method1(&self) -> PyResult<i32> {
        Ok(10)
    }

    fn set_method(&mut self, value: i32) -> PyResult<()> {
        self.num = value;
        Ok(())
    }
}
```

Calls to these methods are protected by the GIL, so both `&self` and `&mut self` can be used.
The return type must be `PyResult<T>` or `T` for some `T` that implements `IntoPy<PyObject>`;
the latter is allowed if the method cannot raise Python exceptions.

A `Python` parameter can be specified as part of method signature, in this case the `py` argument
gets injected by the method wrapper, e.g.

```rust
# use pyo3::prelude::*;
# #[pyclass]
# struct MyClass {
#     num: i32,
#     debug: bool,
# }
#[pymethods]
impl MyClass {
    fn method2(&self, py: Python) -> PyResult<i32> {
        Ok(10)
    }
}
```

From the Python perspective, the `method2` in this example does not accept any arguments.

## Class methods

To create a class method for a custom class, the method needs to be annotated
with the `#[classmethod]` attribute.
This is the equivalent of the Python decorator `@classmethod`.

```rust
# use pyo3::prelude::*;
# use pyo3::types::PyType;
# #[pyclass]
# struct MyClass {
#     num: i32,
#     debug: bool,
# }
#[pymethods]
impl MyClass {
    #[classmethod]
    fn cls_method(cls: &PyType) -> PyResult<i32> {
        Ok(10)
    }
}
```

Declares a class method callable from Python.

* The first parameter is the type object of the class on which the method is called.
  This may be the type object of a derived class.
* The first parameter implicitly has type `&PyType`.
* For details on `parameter-list`, see the documentation of `Method arguments` section.
* The return type must be `PyResult<T>` or `T` for some `T` that implements `IntoPy<PyObject>`.

## Static methods

To create a static method for a custom class, the method needs to be annotated with the
`#[staticmethod]` attribute. The return type must be `T` or `PyResult<T>` for some `T` that implements
`IntoPy<PyObject>`.

```rust
# use pyo3::prelude::*;
# #[pyclass]
# struct MyClass {
#     num: i32,
#     debug: bool,
# }
#[pymethods]
impl MyClass {
    #[staticmethod]
    fn static_method(param1: i32, param2: &str) -> PyResult<i32> {
        Ok(10)
    }
}
```

## Class attributes

To create a class attribute (also called [class variable][classattr]), a method without
any arguments can be annotated with the `#[classattr]` attribute. The return type must be `T` for
some `T` that implements `IntoPy<PyObject>`.

```rust
# use pyo3::prelude::*;
# #[pyclass]
# struct MyClass {}
#[pymethods]
impl MyClass {
    #[classattr]
    fn my_attribute() -> String {
        "hello".to_string()
    }
}

Python::with_gil(|py| {
    let my_class = py.get_type::<MyClass>();
    pyo3::py_run!(py, my_class, "assert my_class.my_attribute == 'hello'")
});
```

Note that unlike class variables defined in Python code, class attributes defined in Rust cannot
be mutated at all:
```rust,ignore
// Would raise a `TypeError: can't set attributes of built-in/extension type 'MyClass'`
pyo3::py_run!(py, my_class, "my_class.my_attribute = 'foo'")
```

If the class attribute is defined with `const` code only, one can also annotate associated
constants:

```rust
# use pyo3::prelude::*;
# #[pyclass]
# struct MyClass {}
#[pymethods]
impl MyClass {
    #[classattr]
    const MY_CONST_ATTRIBUTE: &'static str = "foobar";
}
```

## Callable objects

To specify a custom `__call__` method for a custom class, the method needs to be annotated with
the `#[call]` attribute. Arguments of the method are specified as for instance methods.

```rust
# use pyo3::prelude::*;
use pyo3::types::PyTuple;
# #[pyclass]
# struct MyClass {
#     num: i32,
#     debug: bool,
# }
#[pymethods]
impl MyClass {
    #[call]
    #[args(args="*")]
    fn __call__(&self, args: &PyTuple) -> PyResult<i32> {
        println!("MyClass has been called");
        Ok(self.num)
    }
}
```

## Method arguments

By default, PyO3 uses function signatures to determine which arguments are required. Then it scans
the incoming `args` and `kwargs` parameters. If it can not find all required
parameters, it raises a `TypeError` exception. It is possible to override the default behavior
with the `#[args(...)]` attribute. This attribute accepts a comma separated list of parameters in
the form of `attr_name="default value"`. Each parameter has to match the method parameter by name.

Each parameter can be one of the following types:

 * `"*"`: var arguments separator, each parameter defined after `"*"` is a keyword-only parameter.
   Corresponds to python's `def meth(*, arg1.., arg2=..)`.
 * `args="*"`: "args" is var args, corresponds to Python's `def meth(*args)`. Type of the `args`
   parameter has to be `&PyTuple`.
 * `kwargs="**"`: "kwargs" receives keyword arguments, corresponds to Python's `def meth(**kwargs)`.
   The type of the `kwargs` parameter has to be `Option<&PyDict>`.
 * `arg="Value"`: arguments with default value. Corresponds to Python's `def meth(arg=Value)`.
   If the `arg` argument is defined after var arguments, it is treated as a keyword-only argument.
   Note that `Value` has to be valid rust code, PyO3 just inserts it into the generated
   code unmodified.

Example:
```rust
# use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
#
# #[pyclass]
# struct MyClass {
#     num: i32,
#     debug: bool,
# }
#[pymethods]
impl MyClass {
    #[new]
    #[args(num = "-1", debug = "true")]
    fn new(num: i32, debug: bool) -> Self {
        MyClass { num, debug }
    }

    #[args(
        num = "10",
        debug = "true",
        py_args = "*",
        name = "\"Hello\"",
        py_kwargs = "**"
    )]
    fn method(
        &mut self,
        num: i32,
        debug: bool,
        name: &str,
        py_args: &PyTuple,
        py_kwargs: Option<&PyDict>,
    ) -> PyResult<String> {
        self.debug = debug;
        self.num = num;
        Ok(format!(
            "py_args={:?}, py_kwargs={:?}, name={}, num={}, debug={}",
            py_args, py_kwargs, name, self.num, self.debug
        ))
    }

    fn make_change(&mut self, num: i32, debug: bool) -> PyResult<String> {
        self.num = num;
        self.debug = debug;
        Ok(format!("num={}, debug={}", self.num, self.debug))
    }
}
```
N.B. the position of the `"*"` argument (if included) controls the system of handling positional and keyword arguments. In Python:
```python
import mymodule

mc = mymodule.MyClass()
print(mc.method(44, False, "World", 666, x=44, y=55))
print(mc.method(num=-1, name="World"))
print(mc.make_change(44, False))
print(mc.make_change(debug=False, num=-1))
```
Produces output:
```text
py_args=('World', 666), py_kwargs=Some({'x': 44, 'y': 55}), name=Hello, num=44, debug=false
py_args=(), py_kwargs=None, name=World, num=-1, debug=true
num=44, debug=false
num=-1, debug=false
```

## Implementation details

The `#[pyclass]` macros rely on a lot of conditional code generation: each `#[pyclass]` can optionally have a `#[pymethods]` block as well as several different possible `#[pyproto]` trait implementations.

To support this flexibility the `#[pyclass]` macro expands to a blob of boilerplate code which sets up the structure for ["dtolnay specialization"](https://github.com/dtolnay/case-studies/blob/master/autoref-specialization/README.md). This implementation pattern enables the Rust compiler to use `#[pymethods]` and `#[pyproto]` implementations when they are present, and fall back to default (empty) definitions when they are not.

This simple technique works for the case when there is zero or one implementations. To support multiple `#[pymethods]` for a `#[pyclass]` (in the [`multiple-pymethods`] feature), a registry mechanism provided by the [`inventory`](https://github.com/dtolnay/inventory) crate is used instead. This collects `impl`s at library load time, but isn't supported on all platforms. See [inventory: how it works](https://github.com/dtolnay/inventory#how-it-works) for more details.

The `#[pyclass]` macro expands to roughly the code seen below. The `PyClassImplCollector` is the type used internally by PyO3 for dtolnay specialization:

```rust
# #[cfg(not(feature = "multiple-pymethods"))] {
# use pyo3::prelude::*;
// Note: the implementation differs slightly with the `multiple-pymethods` feature enabled.

/// Class for demonstration
struct MyClass {
    num: i32,
    debug: bool,
}

unsafe impl pyo3::PyTypeInfo for MyClass {
    type AsRefTarget = PyCell<Self>;

    const NAME: &'static str = "MyClass";
    const MODULE: Option<&'static str> = None;

    #[inline]
    fn type_object_raw(py: pyo3::Python) -> *mut pyo3::ffi::PyTypeObject {
        use pyo3::type_object::LazyStaticType;
        static TYPE_OBJECT: LazyStaticType = LazyStaticType::new();
        TYPE_OBJECT.get_or_init::<Self>(py)
    }
}

impl pyo3::pyclass::PyClass for MyClass {
    type Dict = pyo3::pyclass_slots::PyClassDummySlot;
    type WeakRef = pyo3::pyclass_slots::PyClassDummySlot;
    type BaseNativeType = PyAny;
}

impl pyo3::IntoPy<PyObject> for MyClass {
    fn into_py(self, py: pyo3::Python) -> pyo3::PyObject {
        pyo3::IntoPy::into_py(pyo3::Py::new(py, self).unwrap(), py)
    }
}

impl pyo3::class::impl_::PyClassImpl for MyClass {
    const DOC: &'static str = "Class for demonstration";
    const IS_GC: bool = false;
    const IS_BASETYPE: bool = false;
    const IS_SUBCLASS: bool = false;
    type Layout = PyCell<MyClass>;
    type BaseType = PyAny;
    type ThreadChecker = pyo3::class::impl_::ThreadCheckerStub<MyClass>;

    fn for_each_method_def(visitor: &mut dyn FnMut(&[pyo3::class::PyMethodDefType])) {
        use pyo3::class::impl_::*;
        let collector = PyClassImplCollector::<MyClass>::new();
        visitor(collector.py_methods());
        visitor(collector.py_class_descriptors());
        visitor(collector.object_protocol_methods());
        visitor(collector.async_protocol_methods());
        visitor(collector.context_protocol_methods());
        visitor(collector.descr_protocol_methods());
        visitor(collector.mapping_protocol_methods());
        visitor(collector.number_protocol_methods());
    }
    fn get_new() -> Option<pyo3::ffi::newfunc> {
        use pyo3::class::impl_::*;
        let collector = PyClassImplCollector::<Self>::new();
        collector.new_impl()
    }
    fn get_alloc() -> Option<pyo3::ffi::allocfunc> {
        use pyo3::class::impl_::*;
        let collector = PyClassImplCollector::<Self>::new();
        collector.alloc_impl()
    }
    fn get_free() -> Option<pyo3::ffi::freefunc> {
        use pyo3::class::impl_::*;
        let collector = PyClassImplCollector::<Self>::new();
        collector.free_impl()
    }
    fn get_call() -> Option<pyo3::ffi::PyCFunctionWithKeywords> {
        use pyo3::class::impl_::*;
        let collector = PyClassImplCollector::<Self>::new();
        collector.call_impl()
    }
    fn for_each_proto_slot(visitor: &mut dyn FnMut(&[pyo3::ffi::PyType_Slot])) {
        // Implementation which uses dtolnay specialization to load all slots.
        use pyo3::class::impl_::*;
        let collector = PyClassImplCollector::<Self>::new();
        visitor(collector.object_protocol_slots());
        visitor(collector.number_protocol_slots());
        visitor(collector.iter_protocol_slots());
        visitor(collector.gc_protocol_slots());
        visitor(collector.descr_protocol_slots());
        visitor(collector.mapping_protocol_slots());
        visitor(collector.sequence_protocol_slots());
        visitor(collector.async_protocol_slots());
        visitor(collector.buffer_protocol_slots());
    }

    fn get_buffer() -> Option<&'static pyo3::class::impl_::PyBufferProcs> {
        use pyo3::class::impl_::*;
        let collector = PyClassImplCollector::<Self>::new();
        collector.buffer_procs()
    }
}
# Python::with_gil(|py| {
#     let cls = py.get_type::<MyClass>();
#     pyo3::py_run!(py, cls, "assert cls.__name__ == 'MyClass'")
# });
# }
```


[`GILGuard`]: {{#PYO3_DOCS_URL}}/pyo3/struct.GILGuard.html
[`PyTypeInfo`]: {{#PYO3_DOCS_URL}}/pyo3/type_object/trait.PyTypeInfo.html
[`PyTypeObject`]: {{#PYO3_DOCS_URL}}/pyo3/type_object/trait.PyTypeObject.html

[`PyCell`]: {{#PYO3_DOCS_URL}}/pyo3/pycell/struct.PyCell.html
[`PyClass`]: {{#PYO3_DOCS_URL}}/pyo3/pyclass/trait.PyClass.html
[`PyRef`]: {{#PYO3_DOCS_URL}}/pyo3/pycell/struct.PyRef.html
[`PyRefMut`]: {{#PYO3_DOCS_URL}}/pyo3/pycell/struct.PyRefMut.html
[`PyClassInitializer<T>`]: {{#PYO3_DOCS_URL}}/pyo3/pyclass_init/struct.PyClassInitializer.html

[`RefCell`]: https://doc.rust-lang.org/std/cell/struct.RefCell.html

[classattr]: https://docs.python.org/3/tutorial/classes.html#class-and-instance-variables

[`multiple-pymethods`]: features.md#multiple-pymethods
