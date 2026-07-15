use std::collections::VecDeque;
use crate::interpreter::RuntimeError;
use crate::parsing::lexer::*;
use crate::values::Operator;
use crate::values::tape::{Mutator, ROData, TaggedCellPtr};
use thiserror::Error;

macro_rules! expect_token {
    ($self:expr, $pat:pat, $expected:literal) => {
        match $self.consume()? {
            $pat => (),
            other => {
                return Err(SyntaxError::Expected(
                    $expected.to_string(),
                    other.to_string(),
                    $self.row,
                    $self.column,
                ));
            }
        }
    };

    ($self:expr, $pat:pat, $expected:literal, $output:expr) => {
        match $self.consume()? {
            $pat => $output,
            other => {
                return Err(SyntaxError::Expected(
                    $expected.to_string(),
                    other.to_string(),
                    $self.row,
                    $self.column,
                ));
            }
        }
    };
}

#[derive(Clone, Debug)]
pub struct DougChain {
    pub count: usize,
}

#[derive(Clone, Debug)]
pub enum Expression {
    DougSequence(Box<[DougChain]>),
    Variable(String),
    RSequence {
        left: Box<Expression>,
        op: Option<Operator>,
        right: Option<Box<Expression>>,
    },
    Literal(TaggedCellPtr),
}

#[derive(Clone, Debug)]
pub enum ASTNode {
    Doug {
        chains: Box<[DougChain]>,
        reset: bool,
    },
    Tts {
        msg: Option<Expression>,
        use_index: bool,
    },
    Set {
        value: Expression,
        op: Option<Operator>,
    },
    Declare {
        name: String,
        ptr: Box<[DougChain]>,
    },
    Modify {
        name: String,
        op: Option<Operator>,
        value: Expression,
    },
    Loop {
        body: Box<[ASTNode]>,
    },
    Prediction {
        believers_body: Box<[ASTNode]>,
        doubters_body: Box<[ASTNode]>,
        condition: Expression,
    },
    Break,
}
#[derive(Error, Debug)]
pub enum SyntaxError {
    #[error("Expected `{0}` got `{1}` at r:{2},c:{3}")]
    Expected(String, String, u16, u16),

    #[error("Unexpected `{0}` at r:{1},c:{2}")]
    Unexpected(String, u16, u16),

    #[error("Lexing error `{0}` at r:{1},c:{2}")]
    Lexer(LexerError, u16, u16),

    #[error("Cannot print literal")]
    NotPrintable,

    #[error("No more tokens left at r:{0},c:{1}")]
    NoMoreTokens(u16, u16),

    #[error("Breaking from expression tree")]
    BreakFromExprTree,

    #[error("Feature (0) is not in dialect")]
    WrongDialect(String),
}

pub struct Parser<'a> {
    lexer: Option<Lexer<'a>>,
    tokens: VecDeque<Token>,
    column: u16,
    row: u16,
}

impl<'a> Mutator<'a> for Parser<'a> {
    type Scope = ROData<'a>;
    type Input = String;
    type Output = Box<[ASTNode]>;

    fn run(
        &mut self,
        mem: &'a Self::Scope,
        input: Self::Input,
    ) -> Result<Self::Output, crate::interpreter::RuntimeError> {
        self.lexer = Some(Lexer::new(input, mem.clone()));

        self.parse()
            .map_err(|e| RuntimeError::SyntaxError(e.to_string()))
    }
}

impl<'a> Parser<'a> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            column: 0,
            row: 0,
            lexer: None,
            tokens: VecDeque::new(),
        }
    }

    /// Takes a [`Token`] from the bottom of the [`Self::tokens`] and removes it, thus shifting the
    /// bottom to the item before it.
    ///
    /// If there are no items in [`Self::tokens`], we attempt to [`Self::add_line`].
    ///
    /// # Errors
    /// If there are no more tokens, or if we cannot add more lines.
    #[allow(clippy::let_and_return)]
    fn consume(&mut self) -> Result<Token, SyntaxError> {
        self.column += 1;

        while self.tokens.is_empty() {
            self.row += 1;
            self.column = 0;
            self.add_line()?
        }

        let t = self
            .tokens
            .pop_front()
            .ok_or(SyntaxError::NoMoreTokens(self.row, self.column));

        #[cfg(debug_assertions)]
        println!("{t:?}");

        t
    }

    /// Look at the [`Token`] at the bottom of [`Self::tokens`] and don't remove it. Thus this token
    /// will be the one being consumed the next time [`Self::consume`] is called.
    ///
    /// # Errors
    /// Because this doesn't mutate [`self`], this function errors when there are no more tokens
    /// left. This is good for separating out things that are dependent on lines.
    #[allow(clippy::let_and_return)]
    fn peek(&self) -> Result<Token, SyntaxError> {
        let t = self
            .tokens
            .front()
            .cloned()
            .ok_or(SyntaxError::NoMoreTokens(self.row, self.column));

        #[cfg(debug_assertions)]
        println!("Peeking at: {t:?}");

        t
    }

    #[allow(unused)]
    fn peek_two(&self) -> Option<Token> {
        self.tokens.get(1).cloned()
    }

    /// Adds more lines to [`Self::tokens`].
    ///
    /// This function simply uses [`Self::lexer`] to lex more lines from the file before appending
    /// them to [`Self::tokens`].
    ///
    /// # Errors
    /// This fnction only errors out if there is a [`LexerError`], something that should always be
    /// [`LexerError::EOFReached`] or if there is a [`LexerError::InvalidToken`].
    fn add_line(&mut self) -> Result<(), SyntaxError> {
        let tokens = self
            .lexer
            .as_mut()
            .ok_or(SyntaxError::Lexer(
                LexerError::EOFReached,
                self.row,
                self.column,
            ))?
            .lex_line()
            .map_err(|e| SyntaxError::Lexer(e, self.row, self.column))?;
        let tokens: &mut VecDeque<_> = &mut tokens.into_vec().into();
        self.tokens.append(tokens);
        Ok(())
    }

    /// Top level function to parse the entire file
    ///
    /// # Errors
    /// If the `[Self::parse_block]` beneath it fails.
    pub fn parse(&mut self) -> Result<Box<[ASTNode]>, SyntaxError> {
        self.parse_block(true)
    }

    /// Parses a block of nodes. This function takes the [`Token`] inputs from the [`Parser::Lexer`]
    /// and converts them into [`ASTNode`]s. This function loops until we
    /// ([`SyntaxError::NoMoreTokens`])[run out of tokens] or we hit another [`SyntaxError`]. This
    /// exits cleanly if we have no more tokens to consume, and don't expect any, and exit on a `]`
    /// or nothing depending on whether this is the top level or not.
    fn parse_block(&mut self, is_top: bool) -> Result<Box<[ASTNode]>, SyntaxError> {
        let mut nodes = Vec::new();
        self.row += 1;
        self.column = 0;

        while let Ok(token) = self.consume() {
            match token {
                Token::KeyWord(KeyWord::Tts) => {
                    if let Ok(msg) = self.parse_expr() {
                        nodes.push(ASTNode::Tts {
                            msg: Some(msg),
                            use_index: false,
                        });
                    } else {
                        nodes.push(ASTNode::Tts {
                            msg: None,
                            use_index: true,
                        });
                    }
                }
                #[cfg(feature = "xaelia")]
                Token::Paren(ParenThesis::AngleLeft) => {
                    let token2 = self.peek()?;
                    match token2 {
                        Token::Variable(name) => {
                            self.consume()?;
                            let ptr = match self.peek()? {
                                Token::KeyWord(KeyWord::DougChain(_)) => self.parse_doug_expr()?,
                                other => {
                                    return Err(SyntaxError::Expected(
                                        "DougChain".to_string(),
                                        other.to_string(),
                                        self.row,
                                        self.column,
                                    ));
                                }
                            };

                            expect_token!(self, Token::Paren(ParenThesis::AngleRight), ">>");
                            nodes.push(ASTNode::Declare { name, ptr });
                        }
                        other => {
                            return Err(SyntaxError::Expected(
                                "Variable name".to_string(),
                                other.to_string(),
                                self.row,
                                self.column,
                            ));
                        }
                    }
                }

                Token::KeyWord(KeyWord::Set) => {
                    nodes.push(ASTNode::Set {
                        value: self.parse_expr()?,
                        op: None,
                    });
                }
                Token::Operator(op) => {
                    let (value, _) = self.parse_set_expr()?;
                    nodes.push(ASTNode::Set {
                        value,
                        op: Some(op),
                    });
                }
                Token::KeyWord(KeyWord::Loop) => {
                    expect_token!(self, Token::Paren(ParenThesis::SquareLeft), "[");
                    let body = self.parse_block(false)?;
                    nodes.push(ASTNode::Loop { body });
                }
                Token::KeyWord(KeyWord::Prediction) => {
                    let condition = self.parse_expr()?;
                    expect_token!(self, Token::Paren(ParenThesis::SquareLeft), "[");

                    let first_branch_token = self.consume()?;
                    let keyword = match first_branch_token {
                        Token::KeyWord(KeyWord::Believers) => KeyWord::Believers,
                        Token::KeyWord(KeyWord::Doubters) => KeyWord::Doubters,
                        other => {
                            return Err(SyntaxError::Expected(
                                "Believers, Doubters".to_string(),
                                other.to_string(),
                                self.row,
                                self.column,
                            ));
                        }
                    };

                    expect_token!(self, Token::KeyWord(KeyWord::Wins), "win");
                    expect_token!(self, Token::Paren(ParenThesis::SquareLeft), "[");

                    let first_body = self.parse_block(false);
                    expect_token!(self, Token::Paren(ParenThesis::SquareRight), "]");

                    let second_branch_token = self.peek();
                    let second_body = match (second_branch_token, keyword) {
                        (Ok(Token::KeyWord(KeyWord::Doubters)), KeyWord::Doubters)
                        | (Ok(Token::KeyWord(KeyWord::Believers)), KeyWord::Believers) => {
                            return Err(SyntaxError::Expected(
                                "Opposite branches".to_string(),
                                "two of the same".to_string(),
                                self.row,
                                self.column,
                            ));
                        }
                        (Ok(Token::KeyWord(KeyWord::Believers | KeyWord::Doubters)), _) => {
                            let tokens = self.parse_block(false);
                            expect_token!(self, Token::Paren(ParenThesis::SquareRight), "]");
                            tokens
                        }
                        _ => Ok(Vec::new().into()),
                    };

                    let (doubters_body, believers_body) = if let KeyWord::Doubters = keyword {
                        (first_body?, second_body?)
                    } else {
                        (second_body?, first_body?)
                    };

                    nodes.push(ASTNode::Prediction {
                        believers_body,
                        doubters_body,
                        condition,
                    });
                }
                Token::KeyWord(KeyWord::Bald | KeyWord::DougChain { .. }) => {
                    nodes.push(self.parse_doug_node(&token)?);

                    while let Ok(Token::KeyWord(KeyWord::Set) | Token::Operator(_)) = self.peek() {
                        let (value, op) = self.parse_set_expr()?;
                        nodes.push(ASTNode::Set { value, op });
                    }
                }
                Token::KeyWord(KeyWord::Break) => {
                    nodes.push(ASTNode::Break);
                }
                #[cfg(feature = "xaelia")]
                Token::Variable(name) => {
                    let (value, op) = self.parse_set_expr()?;
                    nodes.push(ASTNode::Modify { name, value, op });
                }
                Token::Paren(ParenThesis::Left) => {
                    let chains = self.parse_doug_expr()?;
                    expect_token!(self, Token::Paren(ParenThesis::Right), ")");
                    nodes.push(ASTNode::Doug {
                        chains,
                        reset: true,
                    });
                }
                Token::Paren(ParenThesis::SquareRight) => {
                    if is_top {
                        return Err(SyntaxError::Unexpected(
                            "] in top level".to_string(),
                            self.row,
                            self.column,
                        ));
                    }
                    return Ok(nodes.into());
                }

                other => {
                    return Err(SyntaxError::Unexpected(
                        other.to_string(),
                        self.row,
                        self.column,
                    ));
                }
            }
        }

        if !is_top {
            return Err(SyntaxError::Expected(
                "]".to_string(),
                String::new(),
                self.row,
                self.column,
            ));
        }

        Ok(nodes.into())
    }

    fn parse_doug_node(&mut self, token: &Token) -> Result<ASTNode, SyntaxError> {
        let (mut chains, reset) = match token {
            Token::KeyWord(KeyWord::Bald) => (Vec::new(), true),
            Token::KeyWord(KeyWord::DougChain(count)) => (vec![DougChain { count: *count }], false),
            _ => {
                return Err(SyntaxError::Expected(
                    "Bald, Doug".to_string(),
                    token.to_string(),
                    self.row,
                    self.column,
                ));
            }
        };

        while let Ok(Token::KeyWord(KeyWord::DougChain(count))) = self.peek() {
            self.consume()?;
            chains.push(DougChain { count });
        }

        let chains = chains.into();
        Ok(ASTNode::Doug { chains, reset })
    }

    fn parse_set_expr(&mut self) -> Result<(Expression, Option<Operator>), SyntaxError> {
        match self.consume()? {
            Token::KeyWord(KeyWord::Set) => {
                let value = self.parse_expr()?;
                Ok((value, None))
            }
            Token::Operator(op) => match self.consume()? {
                Token::KeyWord(KeyWord::Set) => {
                    let value = self.parse_expr()?;
                    Ok((value, Some(op)))
                }
                other => Err(SyntaxError::Expected(
                    "set".to_string(),
                    other.to_string(),
                    self.row,
                    self.column,
                )),
            },
            other => Err(SyntaxError::Expected(
                "set, operator".to_string(),
                other.to_string(),
                self.row,
                self.column,
            )),
        }
    }

    fn parse_doug_expr(&mut self) -> Result<Box<[DougChain]>, SyntaxError> {
        let mut dougs = Vec::new();
        while let Ok(token) = self.peek() {
            match token {
                Token::KeyWord(KeyWord::DougChain(count)) => {
                    self.consume()?;
                    dougs.push(DougChain { count });
                }
                Token::Paren(ParenThesis::Right | ParenThesis::AngleRight) => {
                    return Ok(dougs.into());
                }
                _ => {
                    return Err(SyntaxError::Expected(
                        "Doug, Closing Brace".to_string(),
                        token.to_string(),
                        self.row,
                        self.column,
                    ));
                }
            }
        }

        Ok(dougs.into())
    }

    fn parse_expr(&mut self) -> Result<Expression, SyntaxError> {
        let left = match self.peek() {
            Ok(Token::Paren(ParenThesis::Left)) => {
                self.consume()?;
                let expr = self.parse_expr()?;
                expect_token!(self, Token::Paren(ParenThesis::Right), ")");

                Box::new(expr)
            }
            Ok(Token::Literal(lit)) => {
                self.consume()?;
                Box::new(Expression::Literal(lit))
            }
            Ok(Token::KeyWord(KeyWord::DougChain(_))) => {
                Box::new(Expression::DougSequence(self.parse_doug_expr()?))
            }

            Ok(Token::Variable(v)) => {
                self.consume()?;
                Box::new(Expression::Variable(v))
            }

            Ok(Token::Paren(ParenThesis::Right)) => {
                return Err(SyntaxError::BreakFromExprTree);
            }
            Ok(other) => {
                return Err(SyntaxError::Expected(
                    "expression".to_string(),
                    other.to_string(),
                    self.row,
                    self.column,
                ));
            }
            Err(_) => return Err(SyntaxError::BreakFromExprTree),
        };

        #[cfg(feature = "xaelia")]
        let op = match self.peek() {
            Ok(Token::Operator(op)) => {
                if let Some(Token::KeyWord(KeyWord::Set)) = self.peek_two() {
                    None
                } else {
                    self.consume()?;
                    Some(op)
                }
            }
            _ => None,
        };

        #[cfg(not(feature = "xaelia"))]
        let op = match self.peek() {
            Ok(Token::Operator(Operator::Greater)) => Some(Operator::Greater),
            Ok(Token::Operator(Operator::Less)) => Some(Operator::Less),
            Ok(Token::Operator(Operator::LessEquals)) => Some(Operator::LessEquals),
            Ok(Token::Operator(Operator::GreaterEquals)) => Some(Operator::GreaterEquals),
            _ => {
                return Err(SyntaxError::WrongDialect(
                    "C-like expression parsing".to_string(),
                ));
            }
        };

        let Some(op) = op else {
            return Ok(*left);
        };

        let right = match self.peek() {
            Ok(Token::Paren(ParenThesis::Left)) => {
                self.consume()?;
                Some(Box::new(self.parse_expr()?))
            }
            Ok(Token::Literal(lit)) => {
                self.consume()?;
                Some(Box::new(Expression::Literal(lit)))
            }
            Ok(Token::KeyWord(KeyWord::DougChain(_))) => {
                Some(Box::new(Expression::DougSequence(self.parse_doug_expr()?)))
            }
            Ok(Token::Variable(v)) => {
                self.consume()?;
                Some(Box::new(Expression::Variable(v)))
            }

            _ => None,
        };

        Ok(Expression::RSequence {
            left,
            op: Some(op),
            right,
        })
    }
}

impl<'a> Default for Parser<'a> {
    fn default() -> Self {
        Self::new()
    }
}
