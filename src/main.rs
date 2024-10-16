#![allow(dead_code)]
use std::{cmp::Ordering, collections::HashMap, io::Read, ops::ControlFlow};

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{alpha1, alphanumeric1, char, multispace0, multispace1, none_of},
    combinator::{opt, recognize},
    error::ParseError,
    multi::{fold_many0, many0, separated_list0},
    number::complete::recognize_float,
    sequence::{delimited, pair, preceded, terminated},
    Finish, IResult, Parser,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        panic!("Failed to read from stdin");
    }
    let parsed_statements = match statements_finish(&buf) {
        Ok(parsed_statements) => parsed_statements,
        Err(e) => {
            eprintln!("Parse error: {e:?}");
            return Ok(());
        }
    };

    let mut frame = StackFrame::new();
    eval_stmts(&parsed_statements, &mut frame);

    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    F64(f64),
    I64(i64),
    Str(String),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::F64(v) => write!(f, "{v}"),
            Value::I64(v) => write!(f, "{v}"),
            Value::Str(v) => write!(f, "{v}"),
        }
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Value::F64(lhs), Value::F64(rhs)) => lhs.partial_cmp(rhs),
            (Value::I64(lhs), Value::I64(rhs)) => lhs.partial_cmp(rhs),
            (Value::F64(lhs), Value::I64(rhs)) => lhs.partial_cmp(&(*rhs as f64)),
            (Value::I64(lhs), Value::F64(rhs)) => (*lhs as f64).partial_cmp(rhs),
            (Value::Str(lhs), Value::Str(rhs)) => lhs.partial_cmp(rhs),
            _ => None,
        }
    }
}

impl Value {
    fn as_i64(&self) -> Option<i64> {
        match self {
            Self::F64(v) => Some(*v as i64),
            Self::I64(v) => Some(*v),
            Self::Str(v) => v.parse().ok(),
        }
    }
}

fn coerce_f64(a: &Value) -> f64 {
    match a {
        Value::F64(v) => *v,
        Value::I64(v) => *v as f64,
        _ => panic!("Cannot coerce {a:?} to f64"),
    }
}

fn coerce_i64(a: &Value) -> i64 {
    match a {
        Value::F64(v) => *v as i64,
        Value::I64(v) => *v,
        _ => panic!("Cannot coerce {a:?} to i64"),
    }
}

fn coerce_str(a: &Value) -> String {
    match a {
        Value::F64(v) => v.to_string(),
        Value::I64(v) => v.to_string(),
        Value::Str(v) => v.clone(),
    }
}

fn binary_op_str(
    lhs: &Value,
    rhs: &Value,
    d: impl Fn(f64, f64) -> f64,
    i: impl Fn(i64, i64) -> i64,
    s: impl Fn(&str, &str) -> String,
) -> Value {
    match (lhs, rhs) {
        (Value::F64(lhs), rhs) => Value::F64(d(*lhs, coerce_f64(rhs))),
        (lhs, Value::F64(rhs)) => Value::F64(d(coerce_f64(lhs), *rhs)),
        (Value::I64(lhs), Value::I64(rhs)) => Value::I64(i(*lhs, *rhs)),
        (Value::Str(lhs), Value::Str(rhs)) => Value::Str(s(lhs, rhs)),
        _ => {
            panic!("Cannot perform operation on {lhs:?} and {rhs:?}");
        }
    }
}

impl std::ops::Add for Value {
    type Output = Value;
    fn add(self, rhs: Self) -> Self::Output {
        binary_op_str(
            &self,
            &rhs,
            |lhs, rhs| lhs + rhs,
            |lhs, rhs| lhs + rhs,
            |lhs, rhs| lhs.to_owned() + rhs,
        )
    }
}

impl std::ops::Sub for Value {
    type Output = Value;
    fn sub(self, rhs: Self) -> Self::Output {
        binary_op_str(
            &self,
            &rhs,
            |lhs, rhs| lhs - rhs,
            |lhs, rhs| lhs - rhs,
            |_, _| panic!("Strings cannot be subtracted"),
        )
    }
}

impl std::ops::Mul for Value {
    type Output = Value;
    fn mul(self, rhs: Self) -> Self::Output {
        binary_op_str(
            &self,
            &rhs,
            |lhs, rhs| lhs * rhs,
            |lhs, rhs| lhs * rhs,
            |_, _| panic!("Strings cannot be multiplied"),
        )
    }
}

impl std::ops::Div for Value {
    type Output = Value;
    fn div(self, rhs: Self) -> Self::Output {
        binary_op_str(
            &self,
            &rhs,
            |lhs, rhs| lhs / rhs,
            |lhs, rhs| lhs / rhs,
            |_, _| panic!("Strings cannot be divided"),
        )
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum TypeDecl {
    Any,
    F64,
    I64,
    Str,
}

#[derive(Debug)]
pub struct TypeCheckError {
    msg: String,
}

impl std::fmt::Display for TypeCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl TypeCheckError {
    fn new(msg: String) -> Self {
        Self { msg }
    }
}

fn tc_coerce_type(value: &TypeDecl, target: &TypeDecl) -> Result<TypeDecl, TypeCheckError> {
    use TypeDecl::*;
    Ok(match (value, target) {
        (_, Any) => *value,
        (Any, _) => *target,
        (F64 | I64, F64) => F64,
        (F64, I64) => F64,
        (I64, I64) => I64,
        (Str, Str) => Str,
        _ => {
            return Err(TypeCheckError::new(format!(
                "{:?} cannot be coerced to {:?}",
                value, target
            )))
        }
    })
}

pub struct TypeCheckContext<'src> {
    vars: HashMap<&'src str, TypeDecl>,
    funcs: HashMap<String, FnDef<'src>>,
    super_context: Option<&'src TypeCheckContext<'src>>,
}

impl Default for TypeCheckContext<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'src> TypeCheckContext<'src> {
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
            funcs: HashMap::new(),
            super_context: None,
        }
    }

    fn get_var(&self, name: &str) -> Option<TypeDecl> {
        self.vars.get(name).copied()
    }

    fn get_fn(&self, name: &str) -> Option<&FnDef<'src>> {
        if let Some(val) = self.funcs.get(name) {
            Some(val)
        } else if let Some(super_ctx) = self.super_context {
            super_ctx.get_fn(name)
        } else {
            None
        }
    }
}

fn tc_expr<'src>(
    e: &Expression<'src>,
    ctx: &mut TypeCheckContext<'src>,
) -> Result<TypeDecl, TypeCheckError> {
    use Expression::*;
    Ok(match &e {
        NumLiteral(_val) => TypeDecl::F64,
        StrLiteral(_val) => TypeDecl::Str,
        Ident(str) => ctx
            .get_var(str)
            .ok_or_else(|| TypeCheckError::new(format!("Variable {} not found in scope", str)))?,
        _ => todo!(),
    })
}

#[derive(Debug)]
enum BreakResult {
    Return(Value),
    Break,
    Continue,
}

type EvalResult = ControlFlow<BreakResult, Value>;

fn eval_stmts<'src>(stmts: &[Statement<'src>], frame: &mut StackFrame<'src>) -> EvalResult {
    let mut last_result = EvalResult::Continue(Value::I64(0));
    for statement in stmts {
        match statement {
            Statement::Expression(expr) => {
                last_result = EvalResult::Continue(eval(expr, frame)?);
            }
            Statement::VarDef(name, _td, expr) => {
                let value = eval(expr, frame)?;
                frame.vars.insert(name.to_string(), value);
            }
            Statement::VarAssign(name, expr) => {
                if !frame.vars.contains_key(*name) {
                    panic!("Variable {name:?} not found");
                }
                let value = eval(expr, frame)?;
                frame.vars.insert(name.to_string(), value);
            }
            Statement::FnDef {
                name,
                args,
                ret_type,
                stmts,
            } => {
                frame.funcs.insert(
                    name.to_string(),
                    FnDef::User(UserFn {
                        args: args.clone(),
                        ret_type: *ret_type,
                        stmts: stmts.clone(),
                    }),
                );
            }
            Statement::For {
                loop_var,
                start,
                end,
                stmts,
            } => {
                let start = eval(start, frame)?
                    .as_i64()
                    .expect("start must be an integer");
                let end = eval(end, frame)?.as_i64().expect("end must be an integer");
                for i in start..end {
                    frame.vars.insert(loop_var.to_string(), Value::I64(i));
                    match eval_stmts(stmts, frame) {
                        EvalResult::Continue(val) => last_result = EvalResult::Continue(val),
                        EvalResult::Break(BreakResult::Return(val)) => {
                            return EvalResult::Break(BreakResult::Return(val))
                        }
                        EvalResult::Break(BreakResult::Break) => {
                            break;
                        }
                        EvalResult::Break(BreakResult::Continue) => {
                            continue;
                        }
                    }
                }
            }
            Statement::Return(expr) => {
                return EvalResult::Break(BreakResult::Return(eval(expr, frame)?));
            }
            Statement::Break => {
                return EvalResult::Break(BreakResult::Break);
            }
            Statement::Continue => {
                return EvalResult::Break(BreakResult::Continue);
            }
        }
    }
    last_result
}

fn eval<'src>(expr: &Expression<'src>, frame: &mut StackFrame<'src>) -> EvalResult {
    use Expression::*;
    let res = match expr {
        Ident("pi") => Value::F64(std::f64::consts::PI),
        Ident(id) => frame
            .vars
            .get(*id)
            .cloned()
            .unwrap_or_else(|| panic!("Variable {id:?} not found")),
        NumLiteral(n) => Value::F64(*n),
        StrLiteral(s) => Value::Str(s.clone()),
        FnInvoke(name, args) => {
            let mut arg_vals = vec![];
            for arg in args.iter() {
                arg_vals.push(eval(arg, frame)?);
            }

            if let Some(func) = frame.get_fn(name) {
                func.call(&arg_vals, frame)
            } else {
                panic!("Function {name:?} not found");
            }
        }
        Add(lhs, rhs) => eval(lhs, frame)? + eval(rhs, frame)?,
        Sub(lhs, rhs) => eval(lhs, frame)? - eval(rhs, frame)?,
        Mul(lhs, rhs) => eval(lhs, frame)? * eval(rhs, frame)?,
        Div(lhs, rhs) => eval(lhs, frame)? / eval(rhs, frame)?,
        Gt(lhs, rhs) => {
            if eval(lhs, frame)? > eval(rhs, frame)? {
                Value::I64(1)
            } else {
                Value::I64(0)
            }
        }
        Lt(lhs, rhs) => {
            if eval(lhs, frame)? < eval(rhs, frame)? {
                Value::I64(1)
            } else {
                Value::I64(0)
            }
        }
        If(cond, t_case, f_case) => {
            if coerce_i64(&eval(cond, frame)?) != 0 {
                eval_stmts(t_case, frame)?
            } else if let Some(f_case) = f_case {
                eval_stmts(f_case, frame)?
            } else {
                Value::I64(0)
            }
        }
    };
    EvalResult::Continue(res)
}

enum FnDef<'src> {
    User(UserFn<'src>),
    Native(NativeFn<'src>),
}

struct UserFn<'src> {
    args: Vec<(&'src str, TypeDecl)>,
    ret_type: TypeDecl,
    stmts: Statements<'src>,
}

type NativeFnCode = Box<dyn Fn(&[Value]) -> Value>;
struct NativeFn<'src> {
    args: Vec<(&'src str, TypeDecl)>,
    ret_type: TypeDecl,
    code: NativeFnCode,
}

impl<'src> FnDef<'src> {
    fn call(&self, args: &[Value], frame: &StackFrame) -> Value {
        match self {
            Self::User(user_fn) => {
                let mut new_frame = StackFrame::push_stack(frame);
                new_frame.vars = args
                    .iter()
                    .zip(user_fn.args.iter())
                    .map(|(arg, decl)| (decl.0.to_string(), arg.clone()))
                    .collect();
                match eval_stmts(&user_fn.stmts, &mut new_frame) {
                    EvalResult::Continue(val) | EvalResult::Break(BreakResult::Return(val)) => val,
                    EvalResult::Break(BreakResult::Break) => {
                        panic!("Break outside of loop")
                    }
                    EvalResult::Break(BreakResult::Continue) => {
                        panic!("Continue outside of loop")
                    }
                }
            }
            Self::Native(native_fn) => (native_fn.code)(args),
        }
    }

    fn args(&self) -> &Vec<(&'src str, TypeDecl)> {
        match self {
            Self::User(user) => &user.args,
            Self::Native(code) => &code.args,
        }
    }

    fn ret_type(&self) -> &TypeDecl {
        match self {
            Self::User(user) => &user.ret_type,
            Self::Native(native) => &native.ret_type,
        }
    }
}

type Variables = HashMap<String, Value>;
type Functions<'src> = HashMap<String, FnDef<'src>>;

fn print(args: &[Value]) -> Value {
    println!("print: {}", args[0]);
    Value::I64(0)
}

fn p_dgb(args: &[Value]) -> Value {
    println!("dbg: {:?}", args[0]);
    Value::I64(0)
}

struct StackFrame<'src> {
    vars: Variables,
    funcs: Functions<'src>,
    uplevel: Option<&'src StackFrame<'src>>,
}

impl<'src> StackFrame<'src> {
    fn new() -> Self {
        let mut funcs = Functions::new();
        funcs.insert("sqrt".to_string(), unary_fn(f64::sqrt));
        funcs.insert("sin".to_string(), unary_fn(f64::sin));
        funcs.insert("cos".to_string(), unary_fn(f64::cos));
        funcs.insert("tan".to_string(), unary_fn(f64::tan));
        funcs.insert("asin".to_string(), unary_fn(f64::asin));
        funcs.insert("acos".to_string(), unary_fn(f64::acos));
        funcs.insert("atan".to_string(), unary_fn(f64::atan));
        funcs.insert("atan2".to_string(), binary_fn(f64::atan2));
        funcs.insert("pow".to_string(), binary_fn(f64::powf));
        funcs.insert("exp".to_string(), unary_fn(f64::exp));
        funcs.insert("log".to_string(), binary_fn(f64::log));
        funcs.insert("log10".to_string(), unary_fn(f64::log10));
        funcs.insert("print".to_string(), {
            FnDef::Native(NativeFn {
                args: vec![("arg", TypeDecl::Any)],
                ret_type: TypeDecl::Any,
                code: Box::new(print),
            })
        });
        funcs.insert("dbg".to_string(), {
            FnDef::Native(NativeFn {
                args: vec![("arg", TypeDecl::Any)],
                ret_type: TypeDecl::Any,
                code: Box::new(p_dgb),
            })
        });
        funcs.insert("i64".to_string(), {
            FnDef::Native(NativeFn {
                args: vec![("arg", TypeDecl::Any)],
                ret_type: TypeDecl::I64,
                code: Box::new(move |args| {
                    Value::I64(coerce_i64(args.first().expect("function missing argument")))
                }),
            })
        });
        funcs.insert("f64".to_string(), {
            FnDef::Native(NativeFn {
                args: vec![("arg", TypeDecl::Any)],
                ret_type: TypeDecl::F64,
                code: Box::new(move |args| {
                    Value::F64(coerce_f64(args.first().expect("function missing argument")))
                }),
            })
        });
        funcs.insert("str".to_string(), {
            FnDef::Native(NativeFn {
                args: vec![("arg", TypeDecl::Any)],
                ret_type: TypeDecl::Str,
                code: Box::new(move |args| {
                    Value::Str(coerce_str(args.first().expect("function missing argument")))
                }),
            })
        });
        Self {
            vars: Variables::new(),
            funcs,
            uplevel: None,
        }
    }
    fn get_fn(&self, name: &str) -> Option<&FnDef<'src>> {
        let mut next_frame = Some(self);
        while let Some(frame) = next_frame {
            if let Some(func) = frame.funcs.get(name) {
                return Some(func);
            }
            next_frame = frame.uplevel;
        }
        None
    }
    fn push_stack(uplevel: &'src Self) -> Self {
        Self {
            vars: Variables::new(),
            funcs: Functions::new(),
            uplevel: Some(uplevel),
        }
    }
}

fn unary_fn<'a>(f: fn(f64) -> f64) -> FnDef<'a> {
    FnDef::Native(NativeFn {
        args: vec![("arg", TypeDecl::F64)],
        ret_type: TypeDecl::F64,
        code: Box::new(move |args| {
            Value::F64(f(coerce_f64(
                args.iter().next().expect("function missing argument"),
            )))
        }),
    })
}

fn binary_fn<'a>(f: fn(f64, f64) -> f64) -> FnDef<'a> {
    FnDef::Native(NativeFn {
        args: vec![("lhs", TypeDecl::F64), ("rhs", TypeDecl::F64)],
        ret_type: TypeDecl::F64,
        code: Box::new(move |args| {
            let mut args = args.iter();
            let lhs = coerce_f64(
                args.next()
                    .expect("this function is missing the first argument"),
            );
            let rhs = coerce_f64(
                args.next()
                    .expect("this function is missing the second argument"),
            );
            Value::F64(f(lhs, rhs))
        }),
    })
}

#[derive(Debug, PartialEq, Clone)]
enum Expression<'src> {
    Ident(&'src str),
    NumLiteral(f64),
    StrLiteral(String),
    FnInvoke(&'src str, Vec<Expression<'src>>),
    Add(Box<Expression<'src>>, Box<Expression<'src>>),
    Sub(Box<Expression<'src>>, Box<Expression<'src>>),
    Mul(Box<Expression<'src>>, Box<Expression<'src>>),
    Div(Box<Expression<'src>>, Box<Expression<'src>>),
    Gt(Box<Expression<'src>>, Box<Expression<'src>>),
    Lt(Box<Expression<'src>>, Box<Expression<'src>>),
    If(
        Box<Expression<'src>>,
        Box<Statements<'src>>,
        Option<Box<Statements<'src>>>,
    ),
}

#[derive(Debug, PartialEq, Clone)]
enum Statement<'src> {
    Expression(Expression<'src>),
    VarDef(&'src str, TypeDecl, Expression<'src>),
    VarAssign(&'src str, Expression<'src>),
    For {
        loop_var: &'src str,
        start: Expression<'src>,
        end: Expression<'src>,
        stmts: Statements<'src>,
    },
    FnDef {
        name: &'src str,
        args: Vec<(&'src str, TypeDecl)>,
        ret_type: TypeDecl,
        stmts: Statements<'src>,
    },
    Return(Expression<'src>),
    Break,
    Continue,
}

type Statements<'a> = Vec<Statement<'a>>;

fn statements_finish(i: &str) -> Result<Statements, nom::error::Error<&str>> {
    let (_, res) = statements(i).finish()?;
    Ok(res)
}

fn statements(i: &str) -> IResult<&str, Statements> {
    let (i, mut stmts) = many0(statement)(i)?;
    let (i, last) = opt(last_statement)(i)?;
    let (i, _) = opt(multispace0)(i)?;
    if let Some(last) = last {
        stmts.push(last);
    }
    Ok((i, stmts))
}

fn general_statement<'a>(last: bool) -> impl Fn(&'a str) -> IResult<&'a str, Statement> {
    let terminator = move |i| -> IResult<&str, ()> {
        let mut semicolon = pair(tag(";"), multispace0);
        if last {
            Ok((opt(semicolon)(i)?.0, ()))
        } else {
            Ok((semicolon(i)?.0, ()))
        }
    };
    move |input| {
        alt((
            var_def,
            var_assign,
            fn_def_statement,
            for_statement,
            terminated(return_statement, terminator),
            terminated(break_statement, terminator),
            terminated(continue_statement, terminator),
            terminated(expr_statement, terminator),
        ))(input)
    }
}

fn last_statement(i: &str) -> IResult<&str, Statement> {
    general_statement(true)(i)
}

fn statement(i: &str) -> IResult<&str, Statement> {
    general_statement(false)(i)
}

fn for_statement(i: &str) -> IResult<&str, Statement> {
    let (i, _) = space_delimited(tag("for"))(i)?;
    let (i, loop_var) = space_delimited(identifier)(i)?;
    let (i, _) = space_delimited(tag("in"))(i)?;
    let (i, start) = space_delimited(expr)(i)?;
    let (i, _) = space_delimited(tag("to"))(i)?;
    let (i, end) = space_delimited(expr)(i)?;
    let (i, stmts) = delimited(open_brace, statements, close_brace)(i)?;
    Ok((
        i,
        Statement::For {
            loop_var,
            start,
            end,
            stmts,
        },
    ))
}

fn return_statement(i: &str) -> IResult<&str, Statement> {
    let (i, _) = space_delimited(tag("return"))(i)?;
    let (i, expr) = space_delimited(expr)(i)?;
    Ok((i, Statement::Return(expr)))
}

fn break_statement(i: &str) -> IResult<&str, Statement> {
    let (i, _) = space_delimited(tag("break"))(i)?;
    Ok((i, Statement::Break))
}

fn continue_statement(i: &str) -> IResult<&str, Statement> {
    let (i, _) = space_delimited(tag("continue"))(i)?;
    Ok((i, Statement::Continue))
}

fn type_decl(i: &str) -> IResult<&str, TypeDecl> {
    let (i, td) = space_delimited(identifier)(i)?;
    Ok((
        i,
        match td {
            "f64" => TypeDecl::F64,
            "i64" => TypeDecl::I64,
            "str" => TypeDecl::Str,
            _ => {
                panic!("Type annotation has unknown type: {td}")
            }
        },
    ))
}

fn argument(i: &str) -> IResult<&str, (&str, TypeDecl)> {
    let (i, ident) = space_delimited(identifier)(i)?;
    let (i, _) = char(':')(i)?;
    let (i, td) = type_decl(i)?;

    Ok((i, (ident, td)))
}

fn fn_def_statement(i: &str) -> IResult<&str, Statement> {
    let (i, _) = delimited(multispace0, tag("fn"), multispace1)(i)?;
    let (i, name) = space_delimited(identifier)(i)?;
    let (i, _) = space_delimited(tag("("))(i)?;
    let (i, args) = separated_list0(char(','), space_delimited(argument))(i)?;
    let (i, _) = space_delimited(tag(")"))(i)?;
    let (i, _) = space_delimited(tag("->"))(i)?;
    let (i, ret_type) = type_decl(i)?;
    let (i, stmts) = delimited(open_brace, statements, close_brace)(i)?;
    Ok((
        i,
        Statement::FnDef {
            name,
            args,
            ret_type,
            stmts,
        },
    ))
}

fn var_def(i: &str) -> IResult<&str, Statement> {
    let (i, _) = delimited(multispace0, tag("var"), multispace1)(i)?;
    let (i, ident) = space_delimited(identifier)(i)?;
    let (i, _) = space_delimited(char(':'))(i)?;
    let (i, td) = type_decl(i)?;
    let (i, _) = space_delimited(tag("="))(i)?;
    let (i, expr) = space_delimited(expr)(i)?;
    let (i, _) = space_delimited(char(';'))(i)?;
    Ok((i, Statement::VarDef(ident, td, expr)))
}

fn var_assign(i: &str) -> IResult<&str, Statement> {
    let (i, ident) = space_delimited(identifier)(i)?;
    let (i, _) = space_delimited(tag("="))(i)?;
    let (i, expr) = space_delimited(expr)(i)?;
    Ok((i, Statement::VarAssign(ident, expr)))
}

fn expr_statement(i: &str) -> IResult<&str, Statement> {
    let (i, expr) = expr(i)?;
    Ok((i, Statement::Expression(expr)))
}
fn expr(i: &str) -> IResult<&str, Expression> {
    alt((if_expr, cond_expr, num_expr))(i)
}

fn if_expr(i: &str) -> IResult<&str, Expression> {
    let (i, _) = space_delimited(tag("if"))(i)?;
    let (i, cond) = expr(i)?;
    let (i, t_case) = delimited(open_brace, statements, close_brace)(i)?;
    let (i, f_case) = opt(preceded(
        space_delimited(tag("else")),
        delimited(open_brace, statements, close_brace),
    ))(i)?;
    Ok((
        i,
        Expression::If(Box::new(cond), Box::new(t_case), f_case.map(Box::new)),
    ))
}

fn num_expr(i: &str) -> IResult<&str, Expression> {
    let (i, init) = term(i)?;

    fold_many0(
        pair(space_delimited(alt((char('+'), char('-')))), term),
        move || init.clone(),
        |acc, (op, val): (char, Expression)| match op {
            '+' => Expression::Add(Box::new(acc), Box::new(val)),
            '-' => Expression::Sub(Box::new(acc), Box::new(val)),
            _ => panic!("Additive expression should have '+' or '-' operator"),
        },
    )(i)
}

fn term(i: &str) -> IResult<&str, Expression> {
    let (i, init) = factor(i)?;

    fold_many0(
        pair(space_delimited(alt((char('*'), char('/')))), factor),
        move || init.clone(),
        |acc, (op, val): (char, Expression)| match op {
            '*' => Expression::Mul(Box::new(acc), Box::new(val)),
            '/' => Expression::Div(Box::new(acc), Box::new(val)),
            _ => panic!("Multiplicative expression should have '*' or '/' operator"),
        },
    )(i)
}

fn factor(i: &str) -> IResult<&str, Expression> {
    alt((str_literal, num_literal, func_call, ident, parens))(i)
}

fn func_call(i: &str) -> IResult<&str, Expression> {
    let (r, ident) = space_delimited(identifier)(i)?;
    let (r, args) = space_delimited(delimited(
        tag("("),
        many0(delimited(multispace0, expr, space_delimited(opt(tag(","))))),
        tag(")"),
    ))(r)?;
    Ok((r, Expression::FnInvoke(ident, args)))
}

fn space_delimited<'src, O, E>(
    f: impl Parser<&'src str, O, E>,
) -> impl FnMut(&'src str) -> IResult<&'src str, O, E>
where
    E: ParseError<&'src str>,
{
    delimited(multispace0, f, multispace0)
}

fn ident(input: &str) -> IResult<&str, Expression> {
    let (r, res) = space_delimited(identifier)(input)?;
    Ok((r, Expression::Ident(res)))
}

fn identifier(input: &str) -> IResult<&str, &str> {
    recognize(pair(
        alt((alpha1, tag("_"))),
        many0(alt((alphanumeric1, tag("_")))),
    ))(input)
}

fn num_literal(input: &str) -> IResult<&str, Expression> {
    let (r, v) = space_delimited(recognize_float)(input)?;
    Ok((
        r,
        Expression::NumLiteral(v.parse().map_err(|_| {
            nom::Err::Error(nom::error::Error {
                input,
                code: nom::error::ErrorKind::Digit,
            })
        })?),
    ))
}

fn str_literal(i: &str) -> IResult<&str, Expression> {
    let (r0, _) = preceded(multispace0, char('"'))(i)?;
    let (r, val) = many0(none_of("\""))(r0)?;
    let (r, _) = terminated(char('"'), multispace0)(r)?;
    Ok((
        r,
        Expression::StrLiteral(
            val.iter()
                .collect::<String>()
                .replace("\\\\", "\\")
                .replace("\\n", "\n"),
        ),
    ))
}

fn parens(i: &str) -> IResult<&str, Expression> {
    space_delimited(delimited(tag("("), expr, tag(")")))(i)
}

fn cond_expr(i: &str) -> IResult<&str, Expression> {
    let (i, first) = num_expr(i)?;
    let (i, cond) = space_delimited(alt((char('<'), char('>'))))(i)?;
    let (i, second) = num_expr(i)?;
    Ok((
        i,
        match cond {
            '<' => Expression::Lt(Box::new(first), Box::new(second)),
            '>' => Expression::Gt(Box::new(first), Box::new(second)),
            _ => unreachable!(),
        },
    ))
}

fn open_brace(i: &str) -> IResult<&str, ()> {
    let (i, _) = space_delimited(tag("{"))(i)?;
    Ok((i, ()))
}

fn close_brace(i: &str) -> IResult<&str, ()> {
    let (i, _) = space_delimited(tag("}"))(i)?;
    Ok((i, ()))
}
