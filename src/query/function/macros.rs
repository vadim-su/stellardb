//! Declarative macros for defining functions and constants.

/// Macro to define functions and constants in a declarative way.
///
/// # Syntax
///
/// - `fn name(param: Type) -> ReturnType { body }` - regular function
/// - `variadic fn name(args) -> Type { body }` - variadic function (args is `&[Value]`)
/// - `const name: Type = value;` - constant definition
///
/// # Type names
///
/// `Any`, `Null`, `Bool`, `Int`, `Float`, `Numeric`, `String`, `Array`, `Object`
#[macro_export]
macro_rules! define_functions {
    (
        $(
            namespace $ns:ident {
                $(
                    fn $fn_name:ident ( $($param:ident : $param_ty:ident $(?)?),* )
                        -> $ret_ty:ident $(?)?
                        { $($body:tt)* }
                )*
                $(
                    variadic fn $var_fn_name:ident ( $var_args:ident ) -> $var_ret_ty:ident $(?)?
                        { $($var_body:tt)* }
                )*
                $(
                    const $const_name:ident : $const_ty:ident = $const_val:expr;
                )*
            }
        )*
    ) => {
        // Generate static function definitions for regular functions
        $(
            $(
                paste::paste! {
                    #[allow(non_upper_case_globals)]
                    pub static [<$ns:upper _ $fn_name:upper _DEF>]: $crate::query::function::FunctionDef =
                        $crate::query::function::FunctionDef {
                            namespace: stringify!($ns),
                            name: stringify!($fn_name),
                            params: &[
                                $(
                                    $crate::query::function::ParamDef {
                                        name: stringify!($param),
                                        ty: $crate::define_functions!(@type $param_ty),
                                        variadic: false,
                                    },
                                )*
                            ],
                            return_type: $crate::define_functions!(@type $ret_ty),
                            variadic: false,
                            eval: [<eval_ $ns _ $fn_name>],
                        };

                    #[allow(unused_variables)]
                    fn [<eval_ $ns _ $fn_name>](args: &[$crate::document::Value]) -> Result<$crate::document::Value, $crate::query::error::ExecuteError> {
                        // Extract args into named variables
                        let mut _args_iter = args.iter();
                        $(
                            let $param = _args_iter.next().cloned().unwrap_or($crate::document::Value::Null);
                        )*
                        $($body)*
                    }
                }
            )*

            // Generate static function definitions for variadic functions
            $(
                paste::paste! {
                    #[allow(non_upper_case_globals)]
                    pub static [<$ns:upper _ $var_fn_name:upper _DEF>]: $crate::query::function::FunctionDef =
                        $crate::query::function::FunctionDef {
                            namespace: stringify!($ns),
                            name: stringify!($var_fn_name),
                            params: &[],
                            return_type: $crate::define_functions!(@type $var_ret_ty),
                            variadic: true,
                            eval: [<eval_ $ns _ $var_fn_name>],
                        };

                    #[allow(unused_variables)]
                    fn [<eval_ $ns _ $var_fn_name>]($var_args: &[$crate::document::Value]) -> Result<$crate::document::Value, $crate::query::error::ExecuteError> {
                        $($var_body)*
                    }
                }
            )*

            // Generate constant definitions
            $(
                paste::paste! {
                    #[allow(non_upper_case_globals)]
                    pub static [<$ns:upper _ $const_name:upper _DEF>]: $crate::query::function::ConstantDef =
                        $crate::query::function::ConstantDef {
                            namespace: stringify!($ns),
                            name: stringify!($const_name),
                            value_type: $crate::define_functions!(@type $const_ty),
                            value: $crate::define_functions!(@const_value $const_ty, $const_val),
                        };
                }
            )*
        )*

        /// Register all defined functions and constants
        pub fn register_all(registry: &mut $crate::query::function::Registry) {
            $(
                // Register regular functions
                $(
                    paste::paste! {
                        registry.register_function(&[<$ns:upper _ $fn_name:upper _DEF>]);
                    }
                )*
                // Register variadic functions
                $(
                    paste::paste! {
                        registry.register_function(&[<$ns:upper _ $var_fn_name:upper _DEF>]);
                    }
                )*
                // Register constants
                $(
                    paste::paste! {
                        registry.register_constant(&[<$ns:upper _ $const_name:upper _DEF>]);
                    }
                )*
            )*
        }
    };

    // Type parsing helpers
    (@type Any) => { $crate::query::function::FnType::Any };
    (@type Null) => { $crate::query::function::FnType::Null };
    (@type Bool) => { $crate::query::function::FnType::Bool };
    (@type Int) => { $crate::query::function::FnType::Int };
    (@type Float) => { $crate::query::function::FnType::Float };
    (@type Decimal) => { $crate::query::function::FnType::Decimal };
    (@type Numeric) => { $crate::query::function::FnType::Numeric };
    (@type String) => { $crate::query::function::FnType::String };
    (@type Array) => { $crate::query::function::FnType::Array };
    (@type Object) => { $crate::query::function::FnType::Object };

    // Constant value helpers - convert based on declared type
    (@const_value Float, $val:expr) => { $crate::document::Value::Float($val) };
    (@const_value Int, $val:expr) => { $crate::document::Value::Int($val) };
    (@const_value Bool, $val:expr) => { $crate::document::Value::Bool($val) };
    (@const_value String, $val:expr) => { $crate::document::Value::String($val.to_string()) };
}

pub use define_functions;
