use std::{
    fmt::Display,
    ops::{Add, Div, Mul, Sub},
};

use crate::{
    interpreter::RuntimeError,
    values::{
        Operator,
        tape::{AllocObject, LiteralList, TypeList},
    },
};

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum Value {
    String(String),
    Number(f64),
    Boolean(bool),
    Err(RuntimeError),
}

impl Value {
    #[must_use]
    pub fn apply_operator(l: Self, op: Operator, r: Self) -> Self {
        match op {
            Operator::Plus => l + r,
            Operator::Minus => Value::Number(l - r),
            Operator::Multiply => Value::Number(l * r),
            Operator::Divide => Value::Number(l / r),

            Operator::Greater => Value::Boolean(l > r),
            Operator::Less => Value::Boolean(l < r),
            Operator::GreaterEquals => Value::Boolean(l >= r),
            Operator::LessEquals => Value::Boolean(l <= r),
            Operator::Equals => Value::Boolean(l == r),
        }
    }
}

impl Add for Value {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Number(l), Self::Number(r)) => Self::Number(l + r),
            (Self::String(l), Self::Number(r)) => Self::String(l + &r.to_string()),
            (Self::String(l), Self::String(r)) => Self::String(l + &r),
            (l, r) => Self::Err(RuntimeError::BadExpression(
                l.to_string(),
                "+".to_string(),
                r.to_string(),
            )),
        }
    }
}

impl Sub for Value {
    type Output = f64;
    fn sub(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Number(l), Self::Number(r)) => l - r,
            (l, r) => panic!("Invalid expression, {l} - {r}"),
        }
    }
}

impl Mul for Value {
    type Output = f64;

    fn mul(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Number(l), Self::Number(r)) => l * r,
            (l, r) => panic!("Invalid expression, {l} * {r}"),
        }
    }
}

impl Div for Value {
    type Output = f64;

    fn div(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Number(l), Self::Number(r)) => l / r,
            (l, r) => panic!("Invalid expression, {l} / {r}"),
        }
    }
}

impl From<Value> for f64 {
    fn from(value: Value) -> Self {
        match value {
            Value::Number(n) => n,
            Value::Boolean(b) => {
                if b {
                    1f64
                } else {
                    0f64
                }
            }
            s @ (Value::String(_) | Value::Err(_)) => panic!("Cannot use {s} as number"),
        }
    }
}

impl From<Value> for bool {
    fn from(value: Value) -> bool {
        match value {
            Value::Boolean(b) => b,
            Value::Number(n) => n != 0f64,
            Value::String(s) => !s.is_empty(),
            Value::Err(_) => false,
        }
    }
}

impl From<crate::values::tape::Value<'_>> for Value {
    fn from(value: crate::values::tape::Value<'_>) -> Self {
        match value {
            super::tape::Value::Nil => Value::Err(RuntimeError::SegmentationFault),
            super::tape::Value::Array(_a) => Value::Number(0f64),
            super::tape::Value::String(s) => Value::String(s.inner.clone()),
            super::tape::Value::Function(_f) => Value::Number(0f64),
            super::tape::Value::Number(n) => Value::Number(n),
            super::tape::Value::Integer(i) => Value::Number(i as f64),
        }
    }
}

impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(v) => write!(f, "{v}"),
            Self::Number(v) => write!(f, "{v}"),
            Self::Boolean(v) => write!(f, "{v}"),
            Self::Err(v) => write!(f, "{v}"),
        }
    }
}

impl Display for Operator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plus => write!(f, "+"),
            Self::Minus => write!(f, "-"),
            Self::Greater => write!(f, ">"),
            Self::Less => write!(f, "<"),
            Self::GreaterEquals => write!(f, ">="),
            Self::LessEquals => write!(f, "<="),
            Self::Multiply => write!(f, "*"),
            Self::Divide => write!(f, "/"),
            Self::Equals => write!(f, "=="),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Text {
    pub inner: String,
}
impl AllocObject<LiteralList> for Text {
    const TYPE_ID: LiteralList = LiteralList::String;
}
impl AllocObject<TypeList> for Text {
    const TYPE_ID: TypeList = TypeList::String;
}
impl From<String> for Text {
    fn from(value: String) -> Self {
        Self {
            inner: value.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Function {}
#[derive(Debug, Clone, Copy)]
pub struct Array {}
