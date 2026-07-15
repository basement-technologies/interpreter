use std::collections::HashMap;
use thiserror::Error;

use crate::parsing::parser::*;
use crate::values::Value;
use crate::values::tape::{ArraySize, BuildFxHasher, Mutator, MutatorView, TaggedCellPtr};

pub trait TtsEngine {
    fn speak(&self, text: &str);
}

#[derive(Debug, Clone, Copy)]
enum ControlFlow {
    Normal,
    Break,
}

#[derive(Error, Debug, Clone, PartialEq, PartialOrd)]
pub enum RuntimeError {
    #[error("Value {0} is out of range of Tape")]
    OutOfRange(i32),
    #[error("Stack overflow :3")]
    TapeOverFlow,

    #[error("{0} is not defined")]
    NotDefined(String),

    #[error("Expression {0} requires both a left and right value")]
    NoRExpression(String),

    #[error("Could not evaluate expression {0} {1} {2}")]
    BadExpression(String, String, String),

    #[error("Allocation error: {0}")]
    AllocError(String),

    #[error("Syntax error: {0}")]
    SyntaxError(String),

    #[error("Value is nil")]
    SegmentationFault,
}

pub struct Interpreter<'a> {
    parser: Parser<'a>,
    values_idx: i32,

    values_right: HashMap<ArraySize, TaggedCellPtr, BuildFxHasher>,
    values_left: HashMap<ArraySize, TaggedCellPtr, BuildFxHasher>,
    variables: HashMap<String, i32>,
    tts: Option<Box<dyn TtsEngine>>,
}

impl<'a> Interpreter<'a> {
    #[must_use]
    pub fn new(parser: Parser<'a>) -> Self {
        Self {
            parser,
            values_left: HashMap::with_hasher(BuildFxHasher {}),
            values_right: HashMap::with_hasher(BuildFxHasher {}),
            values_idx: 0,
            variables: HashMap::new(),
            tts: None,
        }
    }

    pub fn set_tts(&mut self, engine: Box<dyn TtsEngine>) {
        self.tts = Some(engine);
    }

    /// This function gets a [`value`] from an [`i`] in the tape.
    /// If the [`i`] is less than 0 it used [`Self::values_left`] or else [`Self::values_right`].
    ///
    /// # Errors
    /// If the index is out of range of the tape, this errors evidently. Furthermore, if the length
    /// of the tape somehow exceeds the [`usize::MAX`] it will also error with a
    /// [`RuntimeError::TapeOverFlow`].
    #[allow(clippy::cast_sign_loss)]
    fn get_value(&self, i: i32, mem: &MutatorView) -> Result<Value, RuntimeError> {
        let (container, idx) = if i < 0 {
            (&self.values_left, i.abs() - 1)
        } else {
            (&self.values_right, i)
        };

        mem.get_tape()
            .get_value(idx.cast_unsigned(), mem, container)
            .ok_or(RuntimeError::OutOfRange(i))
    }

    /// This function sets a [`value`] to an [`i`] in the tape.
    /// If the [`i`] is less than 0 it used [`Self::values_left`] or else [`Self::values_right`].
    ///
    /// # Errors
    /// If the index is out of range of the tape, this errors evidently. Furthermore, if the length
    /// of the tape somehow exceeds the [`usize::MAX`] it will also error with a
    /// [`RuntimeError::TapeOverFlow`].
    #[allow(clippy::cast_sign_loss)]
    fn set_value(&mut self, i: i32, value: Value, mem: &MutatorView) -> Result<(), RuntimeError> {
        let (container, idx) = if i < 0 {
            (&mut self.values_left, i.abs() - 1)
        } else {
            (&mut self.values_right, i)
        };

        mem.get_tape()
            .upsert_value(idx.cast_unsigned(), value, container)
    }

    #[allow(unused)]
    pub fn print_state(&self) {
        for (i, v) in self.values_left.iter().enumerate() {
            println!("-{} {:?}", i + 1, v);
        }
        for (i, v) in self.values_right.iter().enumerate() {
            println!("{} {:?}", i + 1, v);
        }
    }

    fn get_doug_notation_index(chains: &[DougChain], start_i: i32) -> i32 {
        let mut res_i = start_i;
        for (i, chain) in chains.iter().enumerate() {
            let value = 1 << (chain.count - 1);

            if i % 2 == 0 {
                res_i += value;
            } else {
                res_i -= value;
            }
        }

        res_i
    }

    /// This function recursively evaluates ([`expr`])[expressions] into a [`Value`].
    /// If the [`expr`] is an [`Expression::Literal`] is simply returns that literal.
    /// If the [`expr`] is an [`Expression::DougSequence`] it  ([`Self::get_value`])[gets the value]
    /// from the tape of what index the chain resolves to.
    /// If it is an [`Expression::Variable`] it gets the variable's value from the tape using the
    /// variable's pointer.
    /// If it is an [`Expression::RSequence`] it evaluates each side of the expression, before
    /// applying the operator between them.
    ///
    /// # Errors
    /// This function errors if a lower level evaluation errors, or if we cannot get the values
    /// requested from the tape.
    fn eval_expression(&self, expr: &Expression, mem: &MutatorView) -> Result<Value, RuntimeError> {
        match expr {
            Expression::Literal(v) => {
                let inner = v.get(mem).get_value();
                Ok(inner.into())
            }
            Expression::DougSequence(s) => self.get_value(Self::get_doug_notation_index(s, 0), mem),
            Expression::RSequence { left, op, right } => {
                let l = self.eval_expression(left, mem)?;
                let r = right.as_ref().map(|r| self.eval_expression(r, mem));
                if let Some(op) = op {
                    let r = r
                        .ok_or(RuntimeError::NoRExpression(format!("{l} {op}")))
                        .flatten()?;

                    Ok(Value::apply_operator(l, *op, r))
                } else {
                    Ok(l)
                }
            }
            Expression::Variable(v) => {
                let val = self
                    .variables
                    .get(v)
                    .ok_or(RuntimeError::NotDefined(v.clone()))?;
                self.get_value(*val, mem)
            }
        }
    }

    /// Interpret a block. This function loops through all the [`nodes`] in the AST provided. It
    /// then evaluates each node, if each evaluation fails it returns a [`RuntimeError`]. If need be
    /// it ([`Self::set_value`])[sets values] into the [`Interpreter`]'s [`Self::values_left`] or
    /// [`Self::values_right`].
    ///
    /// # Errors
    /// If:
    /// - It cannot get the index indicated from the tape.
    /// - It cannot evaluate an expression (if the expr needs both left and right values).
    /// - If it cannot set a value into the tape at the index indicated.
    fn interpret_block(
        &mut self,
        nodes: &[ASTNode],
        mem: &MutatorView,
    ) -> Result<ControlFlow, RuntimeError> {
        for node in nodes {
            #[cfg(debug_assertions)]
            {
                println!("{node:?}");
                self.print_state();
            }

            match node {
                ASTNode::Set { value, op } => match op {
                    None => {
                        self.set_value(self.values_idx, self.eval_expression(value, mem)?, mem)?
                    }
                    Some(op) => {
                        let l = self.get_value(self.values_idx, mem)?;
                        let r = self.eval_expression(value, mem)?;
                        self.set_value(self.values_idx, Value::apply_operator(l, *op, r), mem)?;
                    }
                },

                ASTNode::Modify { name, op, value } => {
                    let idx = self.variables.get(name).expect("Name is not expected");
                    match op {
                        None => self.set_value(*idx, self.eval_expression(value, mem)?, mem)?,
                        Some(op) => {
                            let l = self.get_value(*idx, mem)?;
                            let v =
                                Value::apply_operator(l, *op, self.eval_expression(value, mem)?);
                            self.set_value(*idx, v, mem)?;
                        }
                    }
                }

                ASTNode::Tts { msg, use_index } => {
                    let v = if *use_index {
                        self.get_value(self.values_idx, mem)?
                    } else {
                        self.eval_expression(msg.as_ref().unwrap(), mem)?
                    };
                    let text = format!("{v}");
                    println!("{text}");
                    if let Some(ref engine) = self.tts {
                        engine.speak(&text);
                    }
                }

                ASTNode::Doug { chains, reset } => {
                    self.values_idx = Self::get_doug_notation_index(
                        chains,
                        if *reset { 0 } else { self.values_idx },
                    );
                }
                ASTNode::Break => return Ok(ControlFlow::Break),
                ASTNode::Loop { body } => loop {
                    if let ControlFlow::Break = self.interpret_block(body, mem)? {
                        break;
                    }
                },

                ASTNode::Prediction {
                    believers_body,
                    doubters_body,
                    condition,
                } => {
                    if self.eval_expression(condition, mem)?.into() {
                        if let ControlFlow::Break = self.interpret_block(believers_body, mem)? {
                            return Ok(ControlFlow::Break);
                        }
                    } else if let ControlFlow::Break = self.interpret_block(doubters_body, mem)? {
                        return Ok(ControlFlow::Break);
                    }
                }

                ASTNode::Declare { name, ptr } => {
                    self.values_idx = Self::get_doug_notation_index(ptr, self.values_idx);

                    self.variables.insert(name.clone(), self.values_idx);
                }
            }
        }

        Ok(ControlFlow::Normal)
    }
}

impl<'a> Mutator<'a> for Interpreter<'a> {
    type Input = String;
    type Output = ();
    type Scope = MutatorView<'a>;

    fn run(
        &mut self,
        mem: &'a Self::Scope,
        input: Self::Input,
    ) -> Result<Self::Output, RuntimeError> {
        match self.parser.run(mem.get_data(), input) {
            Ok(nodes) => {
                self.interpret_block(&nodes, mem)?;
            }
            Err(e) => {
                eprintln!("Error: {e}");
            }
        }

        Ok(())
    }
}
