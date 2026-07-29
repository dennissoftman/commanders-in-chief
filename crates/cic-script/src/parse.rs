//! Source text to a syntax tree.
//!
//! # Nesting is bounded, and that is not a style rule
//!
//! The parser is recursive descent, so expression nesting is *call* nesting — and a script is data,
//! which may have arrived in a mod. Without a depth limit, a file consisting of four thousand open
//! parentheses overflows the native stack, and a stack overflow is an abort rather than an error: the
//! process dies with no diagnostic and nothing above can catch it.
//!
//! The interface layer learned the same thing about its layout format and bounded nesting there for
//! the same reason. Leaning on some incidental limit inside a dependency would leave the bound
//! unstated and untested; here it is [`Limits::max_depth`], checked in one place and asserted by a
//! test that builds the pathological file.
//!
//! # Every limit is caller-supplied
//!
//! [`Limits`] follows the convention every decoder in this project uses: an editor loading a
//! campaign's scripts can be generous and a multiplayer client accepting one from a lobby can be
//! strict, running identical code.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Bounds on what a script may contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Largest source text accepted, in bytes.
    pub max_source_bytes: usize,
    /// Largest number of tokens accepted.
    pub max_tokens: usize,
    /// Deepest expression, statement, and block nesting accepted.
    pub max_depth: u32,
    /// Largest number of functions and event handlers accepted.
    pub max_functions: usize,
    /// Largest number of parameters or arguments one call may have.
    pub max_arguments: usize,
}

impl Limits {
    /// Limits sized for ordinary authored content.
    pub const DEFAULT: Self = Self {
        max_source_bytes: 1 << 20,
        max_tokens: 200_000,
        max_depth: 64,
        max_functions: 1_024,
        max_arguments: 16,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A failure turning source into a program.
///
/// Carries a line, because a script author reading "unexpected token" without one has been told
/// nothing they can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    /// Line the failure was found on, counting from one.
    pub line: u32,
    /// What went wrong.
    pub message: String,
}

impl CompileError {
    /// Builds an error at a line.
    pub fn new(line: u32, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

impl Display for CompileError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "line {}: {}", self.line, self.message)
    }
}

impl Error for CompileError {}

/// A lexical token.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Int(i64),
    Number(f64),
    Text(String),
    Keyword(&'static str),
    Symbol(&'static str),
}

/// A token and where it came from.
#[derive(Debug, Clone, PartialEq)]
struct Spanned {
    token: Token,
    line: u32,
}

/// Words the language reserves.
const KEYWORDS: [&str; 10] = [
    "fn", "on", "let", "if", "else", "while", "return", "true", "false", "nil",
];

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A whole-number literal.
    Int(i64),
    /// A real literal.
    Number(f64),
    /// A text literal.
    Text(String),
    /// A truth literal.
    Bool(bool),
    /// The absence of a value.
    Nil,
    /// A named variable.
    Variable(String),
    /// A unary operator applied to one operand.
    Unary {
        /// The operator, as written.
        operator: &'static str,
        /// What it applies to.
        operand: Box<Expr>,
    },
    /// A binary operator applied to two operands.
    Binary {
        /// The operator, as written.
        operator: &'static str,
        /// The left operand.
        left: Box<Expr>,
        /// The right operand.
        right: Box<Expr>,
    },
    /// `&&` or `||`, which are separate because they do not evaluate both sides.
    Logical {
        /// The operator, as written.
        operator: &'static str,
        /// The left operand.
        left: Box<Expr>,
        /// The right operand, evaluated only when the left does not settle the answer.
        right: Box<Expr>,
    },
    /// A call to a function the script defines.
    Call {
        /// Name of the function.
        name: String,
        /// The arguments.
        arguments: Vec<Expr>,
        /// Line the call is on.
        line: u32,
    },
    /// A call to a function the host provides, written `sys.name(...)`.
    HostCall {
        /// Name after the dot.
        name: String,
        /// The arguments.
        arguments: Vec<Expr>,
        /// Line the call is on.
        line: u32,
    },
}

/// A statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// Introduces a local.
    Let {
        /// The local's name.
        name: String,
        /// Its initial value.
        value: Expr,
        /// Line the statement is on.
        line: u32,
    },
    /// Assigns to an existing local.
    Assign {
        /// The local's name.
        name: String,
        /// The new value.
        value: Expr,
        /// Line the statement is on.
        line: u32,
    },
    /// A conditional.
    If {
        /// What decides.
        condition: Expr,
        /// Run when the condition holds.
        then_branch: Vec<Stmt>,
        /// Run when it does not, if anything.
        else_branch: Option<Vec<Stmt>>,
        /// Line the statement is on.
        line: u32,
    },
    /// A loop.
    While {
        /// What decides whether to continue.
        condition: Expr,
        /// The body.
        body: Vec<Stmt>,
        /// Line the statement is on.
        line: u32,
    },
    /// Leaves the function.
    Return {
        /// What to leave with, or nothing.
        value: Option<Expr>,
        /// Line the statement is on.
        line: u32,
    },
    /// An expression evaluated for its effect.
    Expr {
        /// The expression.
        value: Expr,
        /// Line the statement is on.
        line: u32,
    },
}

/// A function or an event handler.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// Its name.
    pub name: String,
    /// Its parameters, in order.
    pub parameters: Vec<String>,
    /// Its body.
    pub body: Vec<Stmt>,
    /// Whether it was declared with `on` rather than `fn`.
    pub is_event: bool,
    /// Line the declaration is on.
    pub line: u32,
}

/// A whole parsed script.
#[derive(Debug, Clone, PartialEq)]
pub struct Ast {
    /// Every function and handler, in source order.
    pub functions: Vec<Function>,
}

/// Parses source text.
///
/// # Errors
///
/// Returns [`CompileError`] for a lexical or syntactic fault, or for any limit in `limits` being
/// crossed.
pub fn parse(source: &str, limits: Limits) -> Result<Ast, CompileError> {
    if source.len() > limits.max_source_bytes {
        return Err(CompileError::new(
            1,
            format!(
                "script of {} bytes exceeds the configured limit {}",
                source.len(),
                limits.max_source_bytes
            ),
        ));
    }
    let tokens = tokenize(source, limits)?;
    Parser {
        tokens,
        position: 0,
        depth: 0,
        limits,
    }
    .program()
}

/// Turns source text into tokens.
#[expect(
    clippy::too_many_lines,
    reason = "a lexer is a flat dispatch over character classes; splitting it would produce \
              functions whose only caller is the next line of this one"
)]
fn tokenize(source: &str, limits: Limits) -> Result<Vec<Spanned>, CompileError> {
    let bytes: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    let mut line = 1u32;

    while index < bytes.len() {
        let character = bytes[index];

        if character == '\n' {
            line += 1;
            index += 1;
            continue;
        }
        if character.is_whitespace() {
            index += 1;
            continue;
        }
        // Comments run to the end of the line. There is no block comment, because an unterminated one
        // silently swallows the rest of a file and the diagnostic points nowhere near the mistake.
        if character == '/' && bytes.get(index + 1) == Some(&'/') {
            while index < bytes.len() && bytes[index] != '\n' {
                index += 1;
            }
            continue;
        }

        if tokens.len() >= limits.max_tokens {
            return Err(CompileError::new(
                line,
                format!("script exceeds the token limit {}", limits.max_tokens),
            ));
        }

        if character.is_ascii_digit() {
            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            // A dot only starts a fraction when a digit follows, so `1.max` would lex as `1` and then
            // `.max` rather than as a malformed number.
            // A dot only starts a fraction when a digit follows, so `1.max` lexes as `1` and then
            // `.max` rather than as a malformed number.
            let mut real = index < bytes.len()
                && bytes[index] == '.'
                && bytes.get(index + 1).is_some_and(char::is_ascii_digit);
            if real {
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
            // An exponent, and only when it is followed by something that makes one. `1e` is an
            // integer and an identifier; without this check it would be a confusing syntax error
            // several tokens later.
            if index < bytes.len() && (bytes[index] == 'e' || bytes[index] == 'E') {
                let mut probe = index + 1;
                if probe < bytes.len() && (bytes[probe] == '+' || bytes[probe] == '-') {
                    probe += 1;
                }
                if bytes.get(probe).is_some_and(char::is_ascii_digit) {
                    index = probe;
                    while index < bytes.len() && bytes[index].is_ascii_digit() {
                        index += 1;
                    }
                    real = true;
                }
            }
            if real {
                let text: String = bytes[start..index].iter().collect();
                tokens.push(Spanned {
                    token: Token::Number(decimal(&text, line)?),
                    line,
                });
            } else {
                let text: String = bytes[start..index].iter().collect();
                let value = text.parse::<i64>().map_err(|_| {
                    CompileError::new(line, format!("integer literal `{text}` is out of range"))
                })?;
                tokens.push(Spanned {
                    token: Token::Int(value),
                    line,
                });
            }
            continue;
        }

        if character.is_alphabetic() || character == '_' {
            let start = index;
            while index < bytes.len() && (bytes[index].is_alphanumeric() || bytes[index] == '_') {
                index += 1;
            }
            let text: String = bytes[start..index].iter().collect();
            let token = KEYWORDS
                .iter()
                .find(|keyword| **keyword == text)
                .map_or_else(
                    || Token::Ident(text.clone()),
                    |keyword| Token::Keyword(keyword),
                );
            tokens.push(Spanned { token, line });
            continue;
        }

        if character == '"' {
            index += 1;
            let mut text = String::new();
            loop {
                let Some(&current) = bytes.get(index) else {
                    return Err(CompileError::new(line, "string literal is not closed"));
                };
                if current == '"' {
                    index += 1;
                    break;
                }
                if current == '\n' {
                    // Refused rather than allowed, because the usual cause is a missing quote and the
                    // alternative is a diagnostic pointing at whatever line the next quote is on.
                    return Err(CompileError::new(line, "string literal is not closed"));
                }
                if current == '\\' {
                    let Some(&escaped) = bytes.get(index + 1) else {
                        return Err(CompileError::new(line, "string literal is not closed"));
                    };
                    text.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        '\\' => '\\',
                        '"' => '"',
                        other => {
                            return Err(CompileError::new(
                                line,
                                format!("unknown escape `\\{other}`"),
                            ));
                        }
                    });
                    index += 2;
                    continue;
                }
                text.push(current);
                index += 1;
            }
            tokens.push(Spanned {
                token: Token::Text(text),
                line,
            });
            continue;
        }

        // Two-character symbols are matched first, or `<=` would lex as `<` then `=`.
        let pair: String = bytes[index..(index + 2).min(bytes.len())].iter().collect();
        if let Some(symbol) = ["==", "!=", "<=", ">=", "&&", "||"]
            .iter()
            .find(|candidate| **candidate == pair)
        {
            tokens.push(Spanned {
                token: Token::Symbol(symbol),
                line,
            });
            index += 2;
            continue;
        }

        let single = character.to_string();
        if let Some(symbol) = [
            "(", ")", "{", "}", ",", ";", ".", "=", "<", ">", "+", "-", "*", "/", "%", "!",
        ]
        .iter()
        .find(|candidate| **candidate == single)
        {
            tokens.push(Spanned {
                token: Token::Symbol(symbol),
                line,
            });
            index += 1;
            continue;
        }

        return Err(CompileError::new(
            line,
            format!("unexpected character `{character}`"),
        ));
    }

    Ok(tokens)
}

/// Parses a decimal literal.
///
/// `str::parse::<f64>` is used, and it is *correctly rounded* — Rust specifies the nearest
/// representable value, so every platform turns the same text into the same bits. That is what makes a
/// literal safe to write in a script that runs inside a lockstep simulation.
///
/// A literal too large to represent is refused rather than becoming an infinity, for the reason
/// [`cic_math::finite`] gives: an infinity turns into a NaN one subtraction later, and every
/// comparison against a NaN is false, so the rule that used it silently does not fire.
fn decimal(text: &str, line: u32) -> Result<f64, CompileError> {
    let value = text
        .parse::<f64>()
        .map_err(|_| CompileError::new(line, format!("number `{text}` is not a valid literal")))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CompileError::new(
            line,
            format!("number `{text}` is too large to represent"),
        ))
    }
}

/// Recursive-descent parser over the token stream.
struct Parser {
    tokens: Vec<Spanned>,
    position: usize,
    depth: u32,
    limits: Limits,
}

impl Parser {
    fn line(&self) -> u32 {
        self.tokens
            .get(self.position)
            .or_else(|| self.tokens.last())
            .map_or(1, |spanned| spanned.line)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position).map(|spanned| &spanned.token)
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position)?.token.clone();
        self.position += 1;
        Some(token)
    }

    fn matches_symbol(&mut self, symbol: &str) -> bool {
        if matches!(self.peek(), Some(Token::Symbol(found)) if *found == symbol) {
            self.position += 1;
            return true;
        }
        false
    }

    fn matches_keyword(&mut self, keyword: &str) -> bool {
        if matches!(self.peek(), Some(Token::Keyword(found)) if *found == keyword) {
            self.position += 1;
            return true;
        }
        false
    }

    fn expect_symbol(&mut self, symbol: &str) -> Result<(), CompileError> {
        if self.matches_symbol(symbol) {
            return Ok(());
        }
        Err(CompileError::new(
            self.line(),
            format!("expected `{symbol}`{}", self.found()),
        ))
    }

    fn expect_ident(&mut self) -> Result<String, CompileError> {
        let line = self.line();
        if let Some(Token::Ident(name)) = self.advance() {
            return Ok(name);
        }
        self.position = self.position.saturating_sub(1);
        Err(CompileError::new(
            line,
            format!("expected a name{}", self.found()),
        ))
    }

    /// Renders what was actually found, for an error message.
    fn found(&self) -> String {
        match self.peek() {
            None => ", but the script ended".to_owned(),
            Some(Token::Ident(name)) => format!(", found `{name}`"),
            Some(Token::Keyword(word)) => format!(", found `{word}`"),
            Some(Token::Symbol(symbol)) => format!(", found `{symbol}`"),
            Some(Token::Int(value)) => format!(", found `{value}`"),
            Some(Token::Number(value)) => format!(", found `{value}`"),
            Some(Token::Text(_)) => ", found a string".to_owned(),
        }
    }

    /// Enters one level of nesting, refusing to go deeper than the limit.
    fn descend(&mut self) -> Result<(), CompileError> {
        self.depth += 1;
        if self.depth > self.limits.max_depth {
            return Err(CompileError::new(
                self.line(),
                format!("nesting deeper than the limit of {}", self.limits.max_depth),
            ));
        }
        Ok(())
    }

    fn ascend(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn program(mut self) -> Result<Ast, CompileError> {
        let mut functions = Vec::new();
        while self.position < self.tokens.len() {
            if functions.len() >= self.limits.max_functions {
                return Err(CompileError::new(
                    self.line(),
                    format!(
                        "more than the limit of {} functions",
                        self.limits.max_functions
                    ),
                ));
            }
            functions.push(self.declaration()?);
        }
        Ok(Ast { functions })
    }

    fn declaration(&mut self) -> Result<Function, CompileError> {
        let line = self.line();
        let is_event = if self.matches_keyword("fn") {
            false
        } else if self.matches_keyword("on") {
            true
        } else {
            return Err(CompileError::new(
                line,
                format!("expected `fn` or `on`{}", self.found()),
            ));
        };

        let name = self.expect_ident()?;
        self.expect_symbol("(")?;
        let mut parameters = Vec::new();
        if !self.matches_symbol(")") {
            loop {
                if parameters.len() >= self.limits.max_arguments {
                    return Err(CompileError::new(
                        self.line(),
                        format!(
                            "more than the limit of {} parameters",
                            self.limits.max_arguments
                        ),
                    ));
                }
                parameters.push(self.expect_ident()?);
                if !self.matches_symbol(",") {
                    break;
                }
            }
            self.expect_symbol(")")?;
        }

        let body = self.block()?;
        Ok(Function {
            name,
            parameters,
            body,
            is_event,
            line,
        })
    }

    fn block(&mut self) -> Result<Vec<Stmt>, CompileError> {
        self.descend()?;
        self.expect_symbol("{")?;
        let mut statements = Vec::new();
        while !self.matches_symbol("}") {
            if self.position >= self.tokens.len() {
                return Err(CompileError::new(
                    self.line(),
                    "expected `}`, but the script ended",
                ));
            }
            statements.push(self.statement()?);
        }
        self.ascend();
        Ok(statements)
    }

    fn statement(&mut self) -> Result<Stmt, CompileError> {
        let line = self.line();

        if self.matches_keyword("let") {
            let name = self.expect_ident()?;
            self.expect_symbol("=")?;
            let value = self.expression()?;
            self.expect_symbol(";")?;
            return Ok(Stmt::Let { name, value, line });
        }
        if self.matches_keyword("if") {
            let condition = self.expression()?;
            let then_branch = self.block()?;
            let else_branch = if self.matches_keyword("else") {
                // `else if` is parsed as an else branch holding one `if`, which keeps the tree shape
                // uniform without a separate node.
                if matches!(self.peek(), Some(Token::Keyword("if"))) {
                    Some(vec![self.statement()?])
                } else {
                    Some(self.block()?)
                }
            } else {
                None
            };
            return Ok(Stmt::If {
                condition,
                then_branch,
                else_branch,
                line,
            });
        }
        if self.matches_keyword("while") {
            let condition = self.expression()?;
            let body = self.block()?;
            return Ok(Stmt::While {
                condition,
                body,
                line,
            });
        }
        if self.matches_keyword("return") {
            let value = if self.matches_symbol(";") {
                None
            } else {
                let value = self.expression()?;
                self.expect_symbol(";")?;
                Some(value)
            };
            return Ok(Stmt::Return { value, line });
        }

        // An assignment is a name followed by `=`, which needs one token of lookahead past the name to
        // tell it from an expression statement that merely starts with one.
        if let Some(Token::Ident(name)) = self.peek().cloned()
            && matches!(
                self.tokens.get(self.position + 1).map(|next| &next.token),
                Some(Token::Symbol("="))
            )
        {
            self.position += 2;
            let value = self.expression()?;
            self.expect_symbol(";")?;
            return Ok(Stmt::Assign { name, value, line });
        }

        let value = self.expression()?;
        self.expect_symbol(";")?;
        Ok(Stmt::Expr { value, line })
    }

    fn expression(&mut self) -> Result<Expr, CompileError> {
        self.descend()?;
        let result = self.logical_or();
        self.ascend();
        result
    }

    fn logical_or(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.logical_and()?;
        while self.matches_symbol("||") {
            let right = self.logical_and()?;
            left = Expr::Logical {
                operator: "||",
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn logical_and(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.equality()?;
        while self.matches_symbol("&&") {
            let right = self.equality()?;
            left = Expr::Logical {
                operator: "&&",
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn equality(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.comparison()?;
        loop {
            let operator = if self.matches_symbol("==") {
                "=="
            } else if self.matches_symbol("!=") {
                "!="
            } else {
                return Ok(left);
            };
            let right = self.comparison()?;
            left = Expr::Binary {
                operator,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn comparison(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.term()?;
        loop {
            let operator = if self.matches_symbol("<=") {
                "<="
            } else if self.matches_symbol(">=") {
                ">="
            } else if self.matches_symbol("<") {
                "<"
            } else if self.matches_symbol(">") {
                ">"
            } else {
                return Ok(left);
            };
            let right = self.term()?;
            left = Expr::Binary {
                operator,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn term(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.factor()?;
        loop {
            let operator = if self.matches_symbol("+") {
                "+"
            } else if self.matches_symbol("-") {
                "-"
            } else {
                return Ok(left);
            };
            let right = self.factor()?;
            left = Expr::Binary {
                operator,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn factor(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.unary()?;
        loop {
            let operator = if self.matches_symbol("*") {
                "*"
            } else if self.matches_symbol("/") {
                "/"
            } else if self.matches_symbol("%") {
                "%"
            } else {
                return Ok(left);
            };
            let right = self.unary()?;
            left = Expr::Binary {
                operator,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn unary(&mut self) -> Result<Expr, CompileError> {
        let operator = if self.matches_symbol("-") {
            "-"
        } else if self.matches_symbol("!") {
            "!"
        } else {
            return self.primary();
        };
        self.descend()?;
        let operand = self.unary()?;
        self.ascend();
        Ok(Expr::Unary {
            operator,
            operand: Box::new(operand),
        })
    }

    fn primary(&mut self) -> Result<Expr, CompileError> {
        let line = self.line();

        if self.matches_symbol("(") {
            self.descend()?;
            let inner = self.expression()?;
            self.ascend();
            self.expect_symbol(")")?;
            return Ok(inner);
        }
        if self.matches_keyword("true") {
            return Ok(Expr::Bool(true));
        }
        if self.matches_keyword("false") {
            return Ok(Expr::Bool(false));
        }
        if self.matches_keyword("nil") {
            return Ok(Expr::Nil);
        }

        match self.advance() {
            Some(Token::Int(value)) => Ok(Expr::Int(value)),
            Some(Token::Number(value)) => Ok(Expr::Number(value)),
            Some(Token::Text(value)) => Ok(Expr::Text(value)),
            Some(Token::Ident(name)) => {
                // `sys` is not a keyword and not a value; it is the only identifier a dot may follow,
                // which is what makes the host surface a closed namespace rather than a field access
                // the language would otherwise have to define.
                if name == "sys" && self.matches_symbol(".") {
                    let host_name = self.expect_ident()?;
                    let arguments = self.arguments()?;
                    return Ok(Expr::HostCall {
                        name: host_name,
                        arguments,
                        line,
                    });
                }
                if matches!(self.peek(), Some(Token::Symbol("("))) {
                    let arguments = self.arguments()?;
                    return Ok(Expr::Call {
                        name,
                        arguments,
                        line,
                    });
                }
                Ok(Expr::Variable(name))
            }
            _ => {
                self.position = self.position.saturating_sub(1);
                Err(CompileError::new(
                    line,
                    format!("expected a value{}", self.found()),
                ))
            }
        }
    }

    fn arguments(&mut self) -> Result<Vec<Expr>, CompileError> {
        self.expect_symbol("(")?;
        let mut arguments = Vec::new();
        if self.matches_symbol(")") {
            return Ok(arguments);
        }
        loop {
            if arguments.len() >= self.limits.max_arguments {
                return Err(CompileError::new(
                    self.line(),
                    format!(
                        "more than the limit of {} arguments",
                        self.limits.max_arguments
                    ),
                ));
            }
            arguments.push(self.expression()?);
            if !self.matches_symbol(",") {
                break;
            }
        }
        self.expect_symbol(")")?;
        Ok(arguments)
    }
}

#[cfg(test)]
mod tests {
    use super::{Ast, Expr, Limits, Stmt, parse};

    fn ast(source: &str) -> Ast {
        parse(source, Limits::DEFAULT).expect("parse")
    }

    #[test]
    fn a_function_and_an_event_handler_are_distinguished() {
        let parsed = ast("fn helper() {} on tick(elapsed) {}");
        assert_eq!(parsed.functions.len(), 2);
        assert!(!parsed.functions[0].is_event);
        assert!(parsed.functions[1].is_event);
        assert_eq!(parsed.functions[1].parameters, vec!["elapsed".to_owned()]);
    }

    #[test]
    fn a_decimal_literal_is_parsed_to_the_nearest_representable_value() {
        // `str::parse::<f64>` is correctly rounded, so every platform turns the same text into the
        // same bits -- which is what makes a literal safe inside a lockstep simulation.
        let parsed = ast("fn f() { let x = 1.5; }");
        let Stmt::Let { value, .. } = &parsed.functions[0].body[0] else {
            panic!("expected a let");
        };
        assert_eq!(*value, Expr::Number(1.5));

        let parsed = ast("fn f() { let x = 0.1; }");
        let Stmt::Let { value, .. } = &parsed.functions[0].body[0] else {
            panic!("expected a let");
        };
        assert_eq!(*value, Expr::Number(0.1), "the nearest f64 to one tenth");
    }

    #[test]
    fn a_literal_too_large_to_represent_is_refused_rather_than_becoming_an_infinity() {
        // An infinity becomes a NaN one subtraction later, and every comparison against a NaN is
        // false -- so the rule that used it would silently not fire.
        let source = format!("fn f() {{ let x = {}.0; }}", "9".repeat(400));
        let error = parse(&source, Limits::DEFAULT).expect_err("refuse");
        assert!(error.message.contains("too large"), "{error}");
    }

    #[test]
    fn precedence_binds_multiplication_tighter_than_addition() {
        let parsed = ast("fn f() { let x = 1 + 2 * 3; }");
        let Stmt::Let { value, .. } = &parsed.functions[0].body[0] else {
            panic!("expected a let");
        };
        let Expr::Binary {
            operator, right, ..
        } = value
        else {
            panic!("expected a binary expression");
        };
        assert_eq!(*operator, "+");
        assert!(
            matches!(**right, Expr::Binary { operator: "*", .. }),
            "the multiplication should be the right operand of the addition"
        );
    }

    #[test]
    fn comparison_binds_tighter_than_the_logical_operators() {
        let parsed = ast("fn f() { let x = 1 < 2 && 3 > 4; }");
        let Stmt::Let { value, .. } = &parsed.functions[0].body[0] else {
            panic!("expected a let");
        };
        assert!(matches!(value, Expr::Logical { operator: "&&", .. }));
    }

    #[test]
    fn a_host_call_is_its_own_node_rather_than_a_field_access() {
        let parsed = ast("fn f() { sys.log(\"hello\"); }");
        let Stmt::Expr { value, .. } = &parsed.functions[0].body[0] else {
            panic!("expected an expression statement");
        };
        let Expr::HostCall {
            name, arguments, ..
        } = value
        else {
            panic!("expected a host call");
        };
        assert_eq!(name, "log");
        assert_eq!(arguments.len(), 1);
    }

    #[test]
    fn an_assignment_is_told_apart_from_an_expression_starting_with_a_name() {
        let parsed = ast("fn f() { x = 1; g(); }");
        assert!(matches!(parsed.functions[0].body[0], Stmt::Assign { .. }));
        assert!(matches!(parsed.functions[0].body[1], Stmt::Expr { .. }));
    }

    #[test]
    fn else_if_parses_as_a_nested_if_rather_than_needing_its_own_node() {
        let parsed = ast("fn f() { if true {} else if false {} else {} }");
        let Stmt::If { else_branch, .. } = &parsed.functions[0].body[0] else {
            panic!("expected an if");
        };
        let branch = else_branch.as_ref().expect("an else");
        assert_eq!(branch.len(), 1);
        assert!(matches!(branch[0], Stmt::If { .. }));
    }

    #[test]
    fn a_comment_runs_to_the_end_of_the_line() {
        let parsed = ast("// leading\nfn f() { // trailing\n let x = 1; }\n// trailing");
        assert_eq!(parsed.functions.len(), 1);
        assert_eq!(parsed.functions[0].body.len(), 1);
    }

    #[test]
    fn four_thousand_open_parentheses_are_an_error_rather_than_a_stack_overflow() {
        // The reason nesting is bounded. A stack overflow is an abort: the process dies with no
        // diagnostic and nothing above can catch it, and a script is data that may have come from a
        // mod.
        let source = format!(
            "fn f() {{ let x = {}1{}; }}",
            "(".repeat(4_000),
            ")".repeat(4_000)
        );
        let error = parse(&source, Limits::DEFAULT).expect_err("refuse");
        assert!(
            error.message.contains("nesting"),
            "expected a nesting error, got: {error}"
        );
    }

    #[test]
    fn deeply_nested_blocks_are_bounded_too() {
        let source = format!(
            "fn f() {{ {} {} }}",
            "if true {".repeat(200),
            "}".repeat(200)
        );
        let error = parse(&source, Limits::DEFAULT).expect_err("refuse");
        assert!(error.message.contains("nesting"), "got: {error}");
    }

    #[test]
    fn every_limit_is_enforced_and_names_itself() {
        let cases: [(&str, Limits, &str); 4] = [
            (
                "fn f() {}",
                Limits {
                    max_source_bytes: 4,
                    ..Limits::DEFAULT
                },
                "bytes",
            ),
            (
                "fn f() { let x = 1; }",
                Limits {
                    max_tokens: 4,
                    ..Limits::DEFAULT
                },
                "token",
            ),
            (
                "fn a() {} fn b() {}",
                Limits {
                    max_functions: 1,
                    ..Limits::DEFAULT
                },
                "functions",
            ),
            (
                "fn f() { g(1, 2, 3); }",
                Limits {
                    max_arguments: 2,
                    ..Limits::DEFAULT
                },
                "arguments",
            ),
        ];
        for (source, limits, expected) in cases {
            let error = parse(source, limits).expect_err("refuse");
            assert!(
                error.message.contains(expected),
                "expected `{expected}` in: {error}"
            );
        }
    }

    #[test]
    fn a_syntax_error_names_the_line_and_what_was_found() {
        let error = parse("fn f() {\n  let x = ;\n}", Limits::DEFAULT).expect_err("refuse");
        assert_eq!(error.line, 2);
        assert!(error.message.contains("expected a value"), "{error}");
        assert!(error.message.contains('`'), "{error}");
    }

    #[test]
    fn an_unclosed_string_is_refused_at_its_own_line() {
        // Refused at the newline rather than run on, so the diagnostic points at the missing quote
        // instead of at whatever line the next quote happens to be on.
        let error = parse("fn f() {\n  sys.log(\"open);\n}", Limits::DEFAULT).expect_err("refuse");
        assert_eq!(error.line, 2);
        assert!(error.message.contains("not closed"), "{error}");
    }

    #[test]
    fn an_unclosed_block_reports_the_end_of_the_script_rather_than_looping() {
        let error = parse("fn f() { let x = 1;", Limits::DEFAULT).expect_err("refuse");
        assert!(error.message.contains("script ended"), "{error}");
    }

    #[test]
    fn an_integer_literal_past_the_range_is_refused() {
        let error =
            parse("fn f() { let x = 99999999999999999999; }", Limits::DEFAULT).expect_err("refuse");
        assert!(error.message.contains("out of range"), "{error}");
    }

    #[test]
    fn an_unknown_escape_is_refused_rather_than_passed_through() {
        let error = parse("fn f() { sys.log(\"a\\qb\"); }", Limits::DEFAULT).expect_err("refuse");
        assert!(error.message.contains("escape"), "{error}");
    }

    #[test]
    fn escapes_that_are_defined_produce_the_characters_they_name() {
        let parsed = ast("fn f() { sys.log(\"a\\nb\\t\\\"c\\\\\"); }");
        let Stmt::Expr { value, .. } = &parsed.functions[0].body[0] else {
            panic!("expected an expression statement");
        };
        let Expr::HostCall { arguments, .. } = value else {
            panic!("expected a host call");
        };
        assert_eq!(arguments[0], Expr::Text("a\nb\t\"c\\".to_owned()));
    }
}
