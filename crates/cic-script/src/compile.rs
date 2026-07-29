//! Syntax tree to bytecode.
//!
//! # Everything resolvable is resolved here
//!
//! A name is looked up once, at compile time, and what the virtual machine executes carries indices.
//! No lookup happens while a script runs — not for a local, not for a function, not for a host call.
//!
//! That is partly speed, and mostly the thing this project keeps returning to: **an error should
//! surface when the file loads, naming the file and the line, rather than when a player happens to
//! trigger the path.** A misspelled host function, a call with the wrong number of arguments, a
//! handler for an event the engine does not define, a variable that was never declared — all of them
//! are compile errors here, and none of them can be a surprise later.
//!
//! # The constant table is interned
//!
//! Two identical literals become one entry. For strings that is load-bearing rather than an economy:
//! a string value *is* its index, so equality is index equality, and without interning two spellings
//! of the same word would compare unequal.

use crate::host::Interface;
use crate::parse::{Ast, CompileError, Expr, Function, Limits, Stmt, parse};
use crate::value::{Value, same_constant};

use std::collections::{BTreeMap, BTreeSet};

/// Most locals one function may have, which is what a one-byte local index affords.
const MAX_LOCALS: usize = 256;

/// One instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    /// Pushes a constant.
    Const(u16),
    /// Pushes nil.
    Nil,
    /// Pushes a truth value.
    Bool(bool),
    /// Pushes a local.
    GetLocal(u8),
    /// Pops into a local.
    SetLocal(u8),
    /// Applies an arithmetic operator to the top two.
    Arithmetic(&'static str),
    /// Applies an ordering operator to the top two.
    Compare(&'static str),
    /// Compares the top two for equality, optionally negated.
    Equal {
        /// Whether the result is inverted, which is `!=`.
        negated: bool,
    },
    /// Negates the top.
    Negate,
    /// Inverts the truth of the top.
    Not,
    /// Discards the top.
    Pop,
    /// Jumps unconditionally.
    Jump(u16),
    /// Pops and jumps when the value was false.
    JumpIfFalse(u16),
    /// Jumps when the top is false, *leaving it there*. For `&&`.
    JumpIfFalseKeep(u16),
    /// Jumps when the top is true, leaving it there. For `||`.
    JumpIfTrueKeep(u16),
    /// Calls a script function.
    Call(u16),
    /// Calls a host function.
    CallHost(u16),
    /// Leaves the function with the top of the stack.
    Return,
}

/// A compiled function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledFunction {
    pub(crate) name: String,
    pub(crate) arity: u8,
    pub(crate) locals: u8,
    pub(crate) code: Vec<Op>,
    /// One line number per instruction, so a runtime error can name where it happened.
    pub(crate) lines: Vec<u32>,
}

/// A compiled script, ready to run.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub(crate) functions: Vec<CompiledFunction>,
    pub(crate) constants: Vec<Value>,
    pub(crate) strings: Vec<String>,
    /// Event name to the function implementing it.
    pub(crate) events: BTreeMap<String, u16>,
    /// How many arguments each host function this program calls takes.
    ///
    /// Recorded here rather than looked up in an [`Interface`] while running, so the machine does not
    /// need the interface at all — and so a program cannot be run against one that disagrees with the
    /// one it was compiled against about how many arguments something takes.
    pub(crate) host_arities: BTreeMap<u16, u8>,
}

impl Program {
    /// Whether the script handles an event.
    #[must_use]
    pub fn handles(&self, event: &str) -> bool {
        self.events.contains_key(event)
    }

    /// Every event the script handles, sorted.
    #[must_use]
    pub fn handled_events(&self) -> Vec<&str> {
        self.events.keys().map(String::as_str).collect()
    }

    /// The program's string table, which a host needs to resolve a [`Value::Str`].
    #[must_use]
    pub fn strings(&self) -> &[String] {
        &self.strings
    }

    /// How many instructions the whole program holds, which is a rough measure of its size.
    #[must_use]
    pub fn instruction_count(&self) -> usize {
        self.functions
            .iter()
            .map(|function| function.code.len())
            .sum()
    }
}

/// Compiles source text against an interface.
///
/// # Errors
///
/// Returns [`CompileError`] for any parse fault, or for a name that does not resolve: an unknown
/// variable, an unknown function, an unknown host function, an event the interface does not define, a
/// call with the wrong number of arguments, or a duplicate declaration.
pub fn compile(
    source: &str,
    interface: &Interface,
    limits: Limits,
) -> Result<Program, CompileError> {
    let ast = parse(source, limits)?;
    compile_ast(&ast, interface)
}

/// Compiles an already-parsed script.
///
/// # Errors
///
/// As [`compile`], minus the parse faults.
pub fn compile_ast(ast: &Ast, interface: &Interface) -> Result<Program, CompileError> {
    // Two passes, because a function may call one declared below it -- and requiring declaration
    // order would make a script's structure depend on its call graph.
    let mut signatures: BTreeMap<String, (u16, u8)> = BTreeMap::new();
    for (position, function) in ast.functions.iter().enumerate() {
        let index = u16::try_from(position).map_err(|_| {
            CompileError::new(function.line, "more functions than an index can address")
        })?;
        let arity = u8::try_from(function.parameters.len()).map_err(|_| {
            CompileError::new(function.line, "more parameters than a call can carry")
        })?;
        if signatures
            .insert(function.name.clone(), (index, arity))
            .is_some()
        {
            return Err(CompileError::new(
                function.line,
                format!("`{}` is declared more than once", function.name),
            ));
        }
        if function.is_event {
            let Some((_, expected)) = interface.event(&function.name) else {
                // The rule that makes a misspelled handler a load error rather than a handler that
                // silently never runs.
                return Err(CompileError::new(
                    function.line,
                    format!(
                        "the engine defines no `{}` event; it defines {}",
                        function.name,
                        list(&interface.event_names())
                    ),
                ));
            };
            if expected != arity {
                return Err(CompileError::new(
                    function.line,
                    format!(
                        "the `{}` event takes {expected} parameters, not {arity}",
                        function.name
                    ),
                ));
            }
        }
    }

    let mut program = Program {
        functions: Vec::new(),
        constants: Vec::new(),
        strings: Vec::new(),
        events: BTreeMap::new(),
        host_arities: BTreeMap::new(),
    };

    for function in &ast.functions {
        let compiled = FunctionCompiler {
            program: &mut program,
            interface,
            signatures: &signatures,
            locals: Vec::new(),
            depth: 0,
            code: Vec::new(),
            lines: Vec::new(),
            line: function.line,
        }
        .compile(function)?;

        if function.is_event {
            let index = u16::try_from(program.functions.len()).map_err(|_| {
                CompileError::new(function.line, "more functions than an index can address")
            })?;
            program.events.insert(function.name.clone(), index);
        }
        program.functions.push(compiled);
    }

    Ok(program)
}

/// Renders a list of names for a diagnostic.
fn list(names: &[&str]) -> String {
    if names.is_empty() {
        return "none".to_owned();
    }
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A local variable in scope.
struct Local {
    name: String,
    depth: u32,
}

/// Compiles one function body.
struct FunctionCompiler<'a> {
    program: &'a mut Program,
    interface: &'a Interface,
    signatures: &'a BTreeMap<String, (u16, u8)>,
    locals: Vec<Local>,
    depth: u32,
    code: Vec<Op>,
    lines: Vec<u32>,
    /// The line of the statement currently being compiled.
    ///
    /// Expressions do not carry a line of their own — an expression can span several — so every
    /// instruction inside one is attributed to the statement that contains it. That is the granularity
    /// a runtime diagnostic needs and the granularity a script author reads.
    line: u32,
}

impl FunctionCompiler<'_> {
    fn compile(mut self, function: &Function) -> Result<CompiledFunction, CompileError> {
        for parameter in &function.parameters {
            self.declare(parameter, function.line)?;
        }
        // A parameter shadowing another is refused rather than silently taking the last one, since
        // the call site cannot tell which it bound.
        let mut seen = BTreeSet::new();
        for parameter in &function.parameters {
            if !seen.insert(parameter.clone()) {
                return Err(CompileError::new(
                    function.line,
                    format!("parameter `{parameter}` is repeated"),
                ));
            }
        }

        // The high-water mark rather than the current count: a block's locals are popped at its end,
        // so the frame has to be sized for the deepest point rather than the final one.
        let mut high_water = self.locals.len();
        self.statements(&function.body, &mut high_water)?;

        // Every function ends by returning nil, which is what a function without a `return` produces
        // and also what stops execution running off the end of the code.
        self.emit(Op::Nil, function.line);
        self.emit(Op::Return, function.line);

        Ok(CompiledFunction {
            name: function.name.clone(),
            arity: u8::try_from(function.parameters.len()).map_err(|_| {
                CompileError::new(function.line, "more parameters than a call can carry")
            })?,
            locals: u8::try_from(high_water.max(function.parameters.len()))
                .map_err(|_| CompileError::new(function.line, "too many locals"))?,
            code: self.code,
            lines: self.lines,
        })
    }

    fn emit(&mut self, op: Op, line: u32) {
        self.code.push(op);
        self.lines.push(line);
    }

    /// Emits a placeholder jump, returning where to patch it.
    fn emit_jump(&mut self, op: Op, line: u32) -> usize {
        self.emit(op, line);
        self.code.len() - 1
    }

    /// Points a previously emitted jump at the current end of the code.
    fn patch(&mut self, at: usize, line: u32) -> Result<(), CompileError> {
        let target = u16::try_from(self.code.len())
            .map_err(|_| CompileError::new(line, "function is too long to jump within"))?;
        match &mut self.code[at] {
            Op::Jump(slot)
            | Op::JumpIfFalse(slot)
            | Op::JumpIfFalseKeep(slot)
            | Op::JumpIfTrueKeep(slot) => *slot = target,
            _ => return Err(CompileError::new(line, "internal: patched a non-jump")),
        }
        Ok(())
    }

    fn declare(&mut self, name: &str, line: u32) -> Result<u8, CompileError> {
        if self.locals.len() >= MAX_LOCALS {
            return Err(CompileError::new(
                line,
                format!("more than {MAX_LOCALS} locals in one function"),
            ));
        }
        let slot = u8::try_from(self.locals.len())
            .map_err(|_| CompileError::new(line, "too many locals"))?;
        self.locals.push(Local {
            name: name.to_owned(),
            depth: self.depth,
        });
        Ok(slot)
    }

    /// Finds a local, searching from the innermost so a shadowing declaration wins.
    fn resolve(&self, name: &str) -> Option<u8> {
        self.locals
            .iter()
            .rposition(|local| local.name == name)
            .and_then(|position| u8::try_from(position).ok())
    }

    fn constant(&mut self, value: Value, line: u32) -> Result<u16, CompileError> {
        // Interned by *bit pattern* rather than by equality, so `0.0` and `-0.0` stay separate
        // entries: they compare equal and they divide differently, and folding them together would
        // change what a program means.
        if let Some(position) = self
            .program
            .constants
            .iter()
            .position(|existing| same_constant(*existing, value))
        {
            return u16::try_from(position)
                .map_err(|_| CompileError::new(line, "too many constants"));
        }
        let index = u16::try_from(self.program.constants.len())
            .map_err(|_| CompileError::new(line, "too many constants"))?;
        self.program.constants.push(value);
        Ok(index)
    }

    fn text_constant(&mut self, text: &str, line: u32) -> Result<u16, CompileError> {
        // Interned, and this is the load-bearing case: a string value *is* its index, so two
        // spellings of the same word must resolve to one entry or `==` reports them different.
        let position = if let Some(found) = self
            .program
            .strings
            .iter()
            .position(|existing| existing == text)
        {
            found
        } else {
            self.program.strings.push(text.to_owned());
            self.program.strings.len() - 1
        };
        let handle =
            u16::try_from(position).map_err(|_| CompileError::new(line, "too many strings"))?;
        self.constant(Value::Str(handle), line)
    }

    fn statements(
        &mut self,
        statements: &[Stmt],
        high_water: &mut usize,
    ) -> Result<(), CompileError> {
        self.depth += 1;
        for statement in statements {
            self.statement(statement, high_water)?;
        }
        self.depth -= 1;
        // Locals declared in this block leave scope. Their slots are reused by the next block, which
        // is why the frame is sized by the high-water mark rather than by the total ever declared.
        while self
            .locals
            .last()
            .is_some_and(|local| local.depth > self.depth)
        {
            self.locals.pop();
        }
        Ok(())
    }

    fn statement(&mut self, statement: &Stmt, high_water: &mut usize) -> Result<(), CompileError> {
        self.line = match statement {
            Stmt::Let { line, .. }
            | Stmt::Assign { line, .. }
            | Stmt::If { line, .. }
            | Stmt::While { line, .. }
            | Stmt::Return { line, .. }
            | Stmt::Expr { line, .. } => *line,
        };
        match statement {
            Stmt::Let { name, value, line } => {
                self.expression(value)?;
                let slot = self.declare(name, *line)?;
                *high_water = (*high_water).max(self.locals.len());
                self.emit(Op::SetLocal(slot), *line);
            }
            Stmt::Assign { name, value, line } => {
                let Some(slot) = self.resolve(name) else {
                    return Err(CompileError::new(
                        *line,
                        format!("`{name}` is not declared; use `let {name} = ...`"),
                    ));
                };
                self.expression(value)?;
                self.emit(Op::SetLocal(slot), *line);
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
                line,
            } => {
                self.expression(condition)?;
                let to_else = self.emit_jump(Op::JumpIfFalse(0), *line);
                self.statements(then_branch, high_water)?;
                if let Some(else_branch) = else_branch {
                    let to_end = self.emit_jump(Op::Jump(0), *line);
                    self.patch(to_else, *line)?;
                    self.statements(else_branch, high_water)?;
                    self.patch(to_end, *line)?;
                } else {
                    self.patch(to_else, *line)?;
                }
            }
            Stmt::While {
                condition,
                body,
                line,
            } => {
                let top = u16::try_from(self.code.len())
                    .map_err(|_| CompileError::new(*line, "function is too long to loop within"))?;
                self.expression(condition)?;
                let to_end = self.emit_jump(Op::JumpIfFalse(0), *line);
                self.statements(body, high_water)?;
                self.emit(Op::Jump(top), *line);
                self.patch(to_end, *line)?;
            }
            Stmt::Return { value, line } => {
                match value {
                    Some(value) => self.expression(value)?,
                    None => self.emit(Op::Nil, *line),
                }
                self.emit(Op::Return, *line);
            }
            Stmt::Expr { value, line } => {
                self.expression(value)?;
                // Every expression leaves one value, and a statement wants none.
                self.emit(Op::Pop, *line);
            }
        }
        Ok(())
    }

    fn expression(&mut self, expression: &Expr) -> Result<(), CompileError> {
        match expression {
            Expr::Int(value) => {
                let index = self.constant(Value::Int(*value), self.line)?;
                self.emit(Op::Const(index), self.line);
            }
            Expr::Number(value) => {
                let index = self.constant(Value::Real(*value), self.line)?;
                self.emit(Op::Const(index), self.line);
            }
            Expr::Text(value) => {
                let index = self.text_constant(value, self.line)?;
                self.emit(Op::Const(index), self.line);
            }
            Expr::Bool(value) => self.emit(Op::Bool(*value), self.line),
            Expr::Nil => self.emit(Op::Nil, self.line),
            Expr::Variable(name) => {
                let Some(slot) = self.resolve(name) else {
                    return Err(CompileError::new(
                        self.line,
                        format!("`{name}` is not declared"),
                    ));
                };
                self.emit(Op::GetLocal(slot), self.line);
            }
            Expr::Unary { operator, operand } => {
                self.expression(operand)?;
                self.emit(
                    if *operator == "-" {
                        Op::Negate
                    } else {
                        Op::Not
                    },
                    self.line,
                );
            }
            Expr::Binary {
                operator,
                left,
                right,
            } => {
                self.expression(left)?;
                self.expression(right)?;
                let op = match *operator {
                    "==" => Op::Equal { negated: false },
                    "!=" => Op::Equal { negated: true },
                    "<" | "<=" | ">" | ">=" => Op::Compare(operator),
                    other => Op::Arithmetic(other),
                };
                self.emit(op, self.line);
            }
            Expr::Logical {
                operator,
                left,
                right,
            } => {
                // Short circuit: the left value is left on the stack and jumped over when it settles
                // the answer, so `a && expensive()` does not evaluate the call.
                self.expression(left)?;
                let jump = if *operator == "&&" {
                    self.emit_jump(Op::JumpIfFalseKeep(0), self.line)
                } else {
                    self.emit_jump(Op::JumpIfTrueKeep(0), self.line)
                };
                self.emit(Op::Pop, self.line);
                self.expression(right)?;
                self.patch(jump, self.line)?;
            }
            Expr::Call {
                name,
                arguments,
                line,
            } => self.script_call(name, arguments, *line)?,
            Expr::HostCall {
                name,
                arguments,
                line,
            } => self.host_call(name, arguments, *line)?,
        }
        Ok(())
    }

    /// Compiles a call to a function the script declares.
    fn script_call(
        &mut self,
        name: &str,
        arguments: &[Expr],
        line: u32,
    ) -> Result<(), CompileError> {
        let Some((index, arity)) = self.signatures.get(name).copied() else {
            return Err(CompileError::new(
                line,
                format!("no function named `{name}` is declared in this script"),
            ));
        };
        Self::check_arity(name, arity, arguments.len(), line)?;
        for argument in arguments {
            self.expression(argument)?;
        }
        self.emit(Op::Call(index), line);
        Ok(())
    }

    /// Compiles a call to a function the engine provides.
    fn host_call(&mut self, name: &str, arguments: &[Expr], line: u32) -> Result<(), CompileError> {
        let Some((index, arity)) = self.interface.function(name) else {
            // The rule the whole module exists for: a mod naming a verb the engine does not define
            // fails to load, rather than failing when somebody triggers it.
            return Err(CompileError::new(
                line,
                format!(
                    "the engine defines no `sys.{name}`; it defines {}",
                    list(&self.interface.function_names())
                ),
            ));
        };
        Self::check_arity(&format!("sys.{name}"), arity, arguments.len(), line)?;
        self.program.host_arities.insert(index, arity);
        for argument in arguments {
            self.expression(argument)?;
        }
        self.emit(Op::CallHost(index), line);
        Ok(())
    }

    fn check_arity(name: &str, expected: u8, found: usize, line: u32) -> Result<(), CompileError> {
        if usize::from(expected) == found {
            return Ok(());
        }
        Err(CompileError::new(
            line,
            format!("`{name}` takes {expected} arguments, not {found}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{Op, compile};
    use crate::host::Interface;
    use crate::parse::Limits;
    use crate::value::Value;

    fn interface() -> Interface {
        let mut interface = Interface::standard();
        interface.declare_event("tick", 1).expect("declare");
        interface.declare_event("start", 0).expect("declare");
        interface
    }

    #[test]
    fn a_script_compiles_and_reports_what_it_handles() {
        let program = compile(
            "on tick(elapsed) { sys.log(\"hi\"); } fn helper() { return 1; } on start() {}",
            &interface(),
            Limits::DEFAULT,
        )
        .expect("compile");
        assert_eq!(program.handled_events(), vec!["start", "tick"]);
        assert!(program.handles("tick"));
        assert!(!program.handles("stop"));
    }

    #[test]
    fn a_host_function_the_engine_does_not_define_fails_to_compile() {
        // The rule that matters most. A mod naming a verb the engine has no implementation of must
        // fail at load, naming the line -- not when somebody eventually triggers it.
        let error = compile(
            "on start() { sys.grant_resources(9999); }",
            &interface(),
            Limits::DEFAULT,
        )
        .expect_err("refuse");
        assert!(error.message.contains("grant_resources"), "{error}");
        assert!(
            error.message.contains("sqrt"),
            "the diagnostic should list what was available: {error}"
        );
    }

    #[test]
    fn an_event_the_engine_does_not_define_fails_to_compile() {
        // Without this a misspelled handler is a handler that silently never runs, which is
        // indistinguishable from one whose body is wrong.
        let error = compile("on tikc(x) {}", &interface(), Limits::DEFAULT).expect_err("refuse");
        assert!(error.message.contains("tikc"), "{error}");
        assert!(error.message.contains("tick"), "{error}");
    }

    #[test]
    fn a_handler_with_the_wrong_number_of_parameters_fails_to_compile() {
        let error = compile("on tick(a, b) {}", &interface(), Limits::DEFAULT).expect_err("refuse");
        assert!(error.message.contains("takes 1 parameters"), "{error}");
    }

    #[test]
    fn a_call_with_the_wrong_number_of_arguments_fails_to_compile() {
        for (source, expected) in [
            ("on start() { sys.sqrt(); }", "sys.sqrt"),
            ("on start() { sys.min(1); }", "sys.min"),
            ("fn f(a) {} on start() { f(1, 2); }", "`f`"),
        ] {
            let error = compile(source, &interface(), Limits::DEFAULT).expect_err("refuse");
            assert!(error.message.contains(expected), "{error}");
            assert!(error.message.contains("takes"), "{error}");
        }
    }

    #[test]
    fn an_undeclared_variable_fails_to_compile() {
        let error = compile(
            "on start() { let x = y + 1; }",
            &interface(),
            Limits::DEFAULT,
        )
        .expect_err("refuse");
        assert!(error.message.contains("`y` is not declared"), "{error}");

        let error =
            compile("on start() { z = 1; }", &interface(), Limits::DEFAULT).expect_err("refuse");
        assert!(error.message.contains("`z` is not declared"), "{error}");
        assert!(error.message.contains("let z"), "{error}");
    }

    #[test]
    fn a_repeated_declaration_fails_to_compile() {
        let error =
            compile("fn f() {} fn f() {}", &interface(), Limits::DEFAULT).expect_err("refuse");
        assert!(error.message.contains("more than once"), "{error}");

        let error = compile("on tick(a) {} fn tick() {}", &interface(), Limits::DEFAULT)
            .expect_err("refuse");
        assert!(error.message.contains("more than once"), "{error}");
    }

    #[test]
    fn a_repeated_parameter_fails_to_compile() {
        let error = compile("fn f(a, a) {}", &interface(), Limits::DEFAULT).expect_err("refuse");
        assert!(error.message.contains("repeated"), "{error}");
    }

    #[test]
    fn a_function_may_call_one_declared_below_it() {
        // Requiring declaration order would make a script's structure depend on its call graph.
        compile(
            "fn first() { return second(); } fn second() { return 1; }",
            &interface(),
            Limits::DEFAULT,
        )
        .expect("compile");
    }

    #[test]
    fn identical_literals_share_one_constant() {
        let program = compile(
            "on start() { sys.log(\"same\"); sys.log(\"same\"); let a = 7; let b = 7; }",
            &interface(),
            Limits::DEFAULT,
        )
        .expect("compile");
        assert_eq!(program.strings, vec!["same".to_owned()]);
        assert_eq!(
            program
                .constants
                .iter()
                .filter(|value| **value == Value::Int(7))
                .count(),
            1
        );
    }

    #[test]
    fn two_strings_that_read_the_same_get_the_same_handle() {
        // Load-bearing rather than an economy: a string value *is* its index, so without interning
        // `"a" == "a"` would be false.
        let program = compile(
            "on start() { sys.log(\"x\"); sys.log(\"x\"); }",
            &interface(),
            Limits::DEFAULT,
        )
        .expect("compile");
        let handles: Vec<Value> = program
            .constants
            .iter()
            .filter(|value| matches!(value, Value::Str(_)))
            .copied()
            .collect();
        assert_eq!(handles, vec![Value::Str(0)]);
    }

    #[test]
    fn a_block_reuses_the_slots_of_a_sibling_block_but_the_frame_fits_the_deepest() {
        let program = compile(
            "on start() { if true { let a = 1; let b = 2; } if true { let c = 3; } }",
            &interface(),
            Limits::DEFAULT,
        )
        .expect("compile");
        let function = &program.functions[0];
        assert_eq!(
            function.locals, 2,
            "the frame must fit the deepest block, not the total ever declared"
        );
    }

    #[test]
    fn a_short_circuit_leaves_the_deciding_value_rather_than_re_evaluating() {
        let program = compile(
            "on start() { let x = true && false; }",
            &interface(),
            Limits::DEFAULT,
        )
        .expect("compile");
        assert!(
            program.functions[0]
                .code
                .iter()
                .any(|op| matches!(op, Op::JumpIfFalseKeep(_))),
            "expected a keeping jump for `&&`"
        );
    }

    #[test]
    fn every_function_ends_in_a_return_so_execution_cannot_run_off_the_end() {
        let program =
            compile("fn f() {} on start() {}", &interface(), Limits::DEFAULT).expect("compile");
        for function in &program.functions {
            assert_eq!(function.code.last(), Some(&Op::Return), "{}", function.name);
        }
    }

    #[test]
    fn an_interface_offering_nothing_refuses_every_host_call() {
        let bare = Interface::new();
        assert!(compile("fn f() { sys.sqrt(1); }", &bare, Limits::DEFAULT).is_err());
    }
}
