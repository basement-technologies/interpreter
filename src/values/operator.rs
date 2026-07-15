#[derive(Clone, Copy, Debug)]
pub enum Operator {
    Plus,
    Minus,
    Multiply,
    Divide,

    Equals,
    Greater,
    GreaterEquals,
    Less,
    LessEquals,
}
