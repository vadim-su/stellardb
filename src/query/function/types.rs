//! Type system for function parameters and return values.

/// Type of a function parameter or return value
#[derive(Debug, Clone, PartialEq)]
pub enum FnType {
    /// Any type (except Null unless Nullable)
    Any,
    /// Null literal
    Null,
    /// Boolean
    Bool,
    /// Integer (i64)
    Int,
    /// Float (f64)
    Float,
    /// Decimal (arbitrary precision)
    Decimal,
    /// Int, Float, or Decimal
    Numeric,
    /// String
    String,
    /// Array of any values
    Array,
    /// Object/Map
    Object,
    /// Union of types: A | B
    Union(Vec<FnType>),
    /// Nullable: Type?
    Nullable(Box<FnType>),
}

impl FnType {
    /// Create a nullable version of this type
    pub fn nullable(self) -> FnType {
        FnType::Nullable(Box::new(self))
    }

    /// Check if a Value matches this type
    pub fn matches(&self, value: &crate::document::Value) -> bool {
        use crate::document::Value;
        match (self, value) {
            (FnType::Any, v) => !matches!(v, Value::Null),
            (FnType::Null, Value::Null) => true,
            (FnType::Bool, Value::Bool(_)) => true,
            (FnType::Int, Value::Int(_)) => true,
            (FnType::Float, Value::Float(_)) => true,
            (FnType::Decimal, Value::Decimal(_)) => true,
            (FnType::Numeric, Value::Int(_) | Value::Float(_) | Value::Decimal(_)) => true,
            (FnType::String, Value::String(_)) => true,
            (FnType::Array, Value::Array(_)) => true,
            (FnType::Object, Value::Object(_)) => true,
            (FnType::Union(types), v) => types.iter().any(|t| t.matches(v)),
            (FnType::Nullable(_), Value::Null) => true,
            (FnType::Nullable(inner), v) => inner.matches(v),
            _ => false,
        }
    }
}

/// Function arity specification
#[derive(Debug, Clone, PartialEq)]
pub enum Arity {
    /// Exactly N arguments
    Fixed(usize),
    /// Between min and max arguments
    Range(usize, usize),
    /// At least N arguments (variadic)
    AtLeast(usize),
}

impl Arity {
    /// Check if argument count is valid
    pub fn check(&self, count: usize) -> bool {
        match self {
            Arity::Fixed(n) => count == *n,
            Arity::Range(min, max) => count >= *min && count <= *max,
            Arity::AtLeast(min) => count >= *min,
        }
    }

    /// Get expected arity description for error messages
    pub fn expected(&self) -> String {
        match self {
            Arity::Fixed(n) => format!("{}", n),
            Arity::Range(min, max) => format!("{}-{}", min, max),
            Arity::AtLeast(min) => format!("at least {}", min),
        }
    }
}

/// Parameter definition
#[derive(Debug, Clone)]
pub struct ParamDef {
    pub name: &'static str,
    pub ty: FnType,
    pub variadic: bool,
}

/// Supported aggregate functions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AggregateFunction {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl AggregateFunction {
    /// Resolve aggregate function name (case-insensitive)
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "count" => Some(Self::Count),
            "sum" => Some(Self::Sum),
            "avg" => Some(Self::Avg),
            "min" => Some(Self::Min),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}
