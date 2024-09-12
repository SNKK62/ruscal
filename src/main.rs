use std::{collections::HashMap, io::Read};

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{alpha1, alphanumeric1, char, multispace0, multispace1},
    combinator::{opt, recognize},
    error::ParseError,
    multi::{fold_many0, many0, separated_list0},
    number::complete::recognize_float,
    sequence::{delimited, pair, preceded, terminated},
    Finish, IResult, Parser,
};

fn main() {
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        panic!("Failed to read from stdin");
    }
    let parsed_statements = match statements_finish(&buf) {
        Ok(parsed_statements) => parsed_statements,
        Err(e) => {
            eprintln!("Parse error: {e:?}");
            return;
        }
    };

    let mut frame = StackFrame::new();
    eval_stmts(&parsed_statements, &mut frame);
}

fn eval_stmts<'src>(stmts: &[Statement<'src>], frame: &mut StackFrame<'src>) -> f64 {
    let mut last_result = 0.;
    for statement in stmts {
        match statement {
            Statement::Expression(expr) => last_result = eval(expr, frame),
            Statement::VarDef(name, expr) => {
                let value = eval(expr, frame);
                frame.vars.insert(name.to_string(), value);
            }
            Statement::VarAssign(name, expr) => {
                if !frame.vars.contains_key(*name) {
                    panic!("Variable {name:?} not found");
                }
                let value = eval(expr, frame);
                frame.vars.insert(name.to_string(), value);
            }
            Statement::FnDef { name, args, stmts } => {
                frame.funcs.insert(
                    name.to_string(),
                    FnDef::User(UserFn {
                        args: args.clone(),
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
                let start = eval(start, frame) as isize;
                let end = eval(end, frame) as isize;
                for i in start..end {
                    frame.vars.insert(loop_var.to_string(), i as f64);
                    eval_stmts(stmts, frame);
                }
            }
        }
    }
    last_result
}

fn eval(expr: &Expression, frame: &StackFrame) -> f64 {
    use Expression::*;
    match expr {
        Ident("pi") => std::f64::consts::PI,
        Ident(id) => *frame.vars.get(*id).expect("Variable not found"),
        NumLiteral(n) => *n,
        FnInvoke(name, args) => {
            if let Some(func) = frame.get_fn(name) {
                let args: Vec<_> = args.iter().map(|arg| eval(arg, frame)).collect();
                func.call(&args, frame)
            } else {
                panic!("Function {name:?} not found");
            }
        }
        Add(lhs, rhs) => eval(lhs, frame) + eval(rhs, frame),
        Sub(lhs, rhs) => eval(lhs, frame) - eval(rhs, frame),
        Mul(lhs, rhs) => eval(lhs, frame) * eval(rhs, frame),
        Div(lhs, rhs) => eval(lhs, frame) / eval(rhs, frame),
        If(cond, t_case, f_case) => {
            if eval(cond, frame) != 0. {
                eval(t_case, frame)
            } else if let Some(f_case) = f_case {
                eval(f_case, frame)
            } else {
                0.
            }
        }
    }
}

enum FnDef<'src> {
    User(UserFn<'src>),
    Native(NativeFn),
}

struct UserFn<'src> {
    args: Vec<&'src str>,
    stmts: Statements<'src>,
}

struct NativeFn {
    code: Box<dyn Fn(&[f64]) -> f64>,
}

impl<'src> FnDef<'src> {
    fn call(&self, args: &[f64], frame: &StackFrame) -> f64 {
        match self {
            Self::User(user_fn) => {
                let mut new_frame = StackFrame::push_stack(frame);
                new_frame.vars = args
                    .iter()
                    .zip(user_fn.args.iter())
                    .map(|(args, name)| (name.to_string(), *args))
                    .collect();
                new_frame.uplevel = Some(frame);
                eval_stmts(&user_fn.stmts, &mut new_frame)
            }
            Self::Native(native_fn) => (native_fn.code)(args),
        }
    }
}

type Variables = HashMap<String, f64>;
type Functions<'src> = HashMap<String, FnDef<'src>>;

fn print(arg: f64) -> f64 {
    println!("print: {arg}");
    0.
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
        funcs.insert("print".to_string(), unary_fn(print));
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
        code: Box::new(move |args| {
            f(*args
                .iter()
                .next()
                .expect("this function is missing an argument"))
        }),
    })
}

fn binary_fn<'a>(f: fn(f64, f64) -> f64) -> FnDef<'a> {
    FnDef::Native(NativeFn {
        code: Box::new(move |args| {
            let mut args = args.iter();
            let lhs = args
                .next()
                .expect("this function is missing the first argument");
            let rhs = args
                .next()
                .expect("this function is missing the second argument");
            f(*lhs, *rhs)
        }),
    })
}

#[derive(Debug, PartialEq, Clone)]
enum Expression<'src> {
    Ident(&'src str),
    NumLiteral(f64),
    FnInvoke(&'src str, Vec<Expression<'src>>),
    Add(Box<Expression<'src>>, Box<Expression<'src>>),
    Sub(Box<Expression<'src>>, Box<Expression<'src>>),
    Mul(Box<Expression<'src>>, Box<Expression<'src>>),
    Div(Box<Expression<'src>>, Box<Expression<'src>>),
    If(
        Box<Expression<'src>>,
        Box<Expression<'src>>,
        Option<Box<Expression<'src>>>,
    ),
}

#[derive(Debug, PartialEq, Clone)]
enum Statement<'src> {
    Expression(Expression<'src>),
    VarDef(&'src str, Expression<'src>),
    VarAssign(&'src str, Expression<'src>),
    For {
        loop_var: &'src str,
        start: Expression<'src>,
        end: Expression<'src>,
        stmts: Statements<'src>,
    },
    FnDef {
        name: &'src str,
        args: Vec<&'src str>,
        stmts: Statements<'src>,
    },
}

type Statements<'a> = Vec<Statement<'a>>;

fn statements_finish(i: &str) -> Result<Statements, nom::error::Error<&str>> {
    let (_, res) = statements(i).finish()?;
    Ok(res)
}

fn statements(i: &str) -> IResult<&str, Statements> {
    let (i, stmts) = many0(statement)(i)?;
    let (i, _) = opt(char(';'))(i)?;
    Ok((i, stmts))
}

fn statement(i: &str) -> IResult<&str, Statement> {
    alt((
        for_statement,
        fn_def_statement,
        terminated(alt((var_def, var_assign, expr_statement)), char(';')),
    ))(i)
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

fn fn_def_statement(i: &str) -> IResult<&str, Statement> {
    let (i, _) = delimited(multispace0, tag("fn"), multispace1)(i)?;
    let (i, ident) = space_delimited(identifier)(i)?;
    let (i, _) = space_delimited(tag("("))(i)?;
    let (i, args) = separated_list0(char(','), space_delimited(identifier))(i)?;
    let (i, _) = space_delimited(tag(")"))(i)?;
    let (i, stmts) = delimited(open_brace, statements, close_brace)(i)?;
    Ok((
        i,
        Statement::FnDef {
            name: ident,
            args,
            stmts,
        },
    ))
}

fn var_def(i: &str) -> IResult<&str, Statement> {
    let (i, _) = delimited(multispace0, tag("var"), multispace1)(i)?;
    let (i, ident) = space_delimited(identifier)(i)?;
    let (i, _) = space_delimited(tag("="))(i)?;
    let (i, expr) = space_delimited(expr)(i)?;
    Ok((i, Statement::VarDef(ident, expr)))
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
    alt((if_expr, num_expr))(i)
}

fn if_expr(i: &str) -> IResult<&str, Expression> {
    let (i, _) = space_delimited(tag("if"))(i)?;
    let (i, cond) = expr(i)?;
    let (i, t_case) = delimited(open_brace, expr, close_brace)(i)?;
    let (i, f_case) = opt(preceded(
        space_delimited(tag("else")),
        delimited(open_brace, expr, close_brace),
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
    alt((number, func_call, ident, parens))(i)
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

fn number(input: &str) -> IResult<&str, Expression> {
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

fn parens(i: &str) -> IResult<&str, Expression> {
    space_delimited(delimited(tag("("), expr, tag(")")))(i)
}

fn open_brace(i: &str) -> IResult<&str, ()> {
    let (i, _) = space_delimited(tag("{"))(i)?;
    Ok((i, ()))
}

fn close_brace(i: &str) -> IResult<&str, ()> {
    let (i, _) = space_delimited(tag("}"))(i)?;
    Ok((i, ()))
}
