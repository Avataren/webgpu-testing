//! A small expression language for conditions, scores and values in brains, so trees
//! and state machines written as data (JSON, or Lua tables without functions) can still
//! say `hunger > 0.6 and not group.alarm` or `dist(target) < 2`.
//!
//! - Values: numbers, `'text'` or `"text"`, `true`, `false`, `nil`.
//! - Names read the blackboard: `hunger` (the agent's), `group.alarm` (its group's),
//!   `global.night` (the scene's); `spot.kind` reads a field of a value; `self` is the
//!   agent.
//! - Operators, loosest first: `or` (`||`), `and` (`&&`), `not` (`!`), comparisons
//!   (`==`, `~=` or `!=`, `<`, `<=`, `>`, `>=`), `..` (join as text), `+ -`, `* / %`,
//!   unary `-`. `and` / `or` work as in Lua (`x or 3` is 3 when x is nil).
//! - Functions: `dist(a[, b])` (metres from the agent, or between two positions or
//!   entities), `sees(tag)`, `heard(tag)`, `count(tag[, radius])` (what the agent
//!   perceives; `tag` may be omitted), `nearest(tag)` (an entity or nil),
//!   `in_state(name)`, `time()`, `state_time()`, `random()`, `random(a, b)`,
//!   `chance(p)`, `abs`, `min`, `max`, `clamp(x, a, b)`, `floor`, `ceil`, `sqrt`,
//!   `exists(x)`, `len(x)`.
//!
//! Arithmetic on values that are not numbers gives nil, and ordering comparisons with
//! them are false, so a missing key never stops a brain.

use std::fmt;
use std::sync::Arc;

use super::value::Value;

/// Functions an expression can call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Function {
    Dist,
    Sees,
    Heard,
    Count,
    Nearest,
    InState,
    Time,
    StateTime,
    Random,
    Chance,
    Abs,
    Min,
    Max,
    Clamp,
    Floor,
    Ceil,
    Sqrt,
    Exists,
    Len,
}

impl Function {
    fn from_name(name: &str) -> Option<(Self, usize, usize)> {
        // (function, fewest arguments, most arguments)
        Some(match name {
            "dist" => (Function::Dist, 1, 2),
            "sees" => (Function::Sees, 0, 1),
            "heard" => (Function::Heard, 0, 1),
            "count" => (Function::Count, 0, 2),
            "nearest" => (Function::Nearest, 0, 1),
            "in_state" => (Function::InState, 1, 1),
            "time" => (Function::Time, 0, 0),
            "state_time" => (Function::StateTime, 0, 0),
            "random" => (Function::Random, 0, 2),
            "chance" => (Function::Chance, 1, 1),
            "abs" => (Function::Abs, 1, 1),
            "min" => (Function::Min, 2, 8),
            "max" => (Function::Max, 2, 8),
            "clamp" => (Function::Clamp, 3, 3),
            "floor" => (Function::Floor, 1, 1),
            "ceil" => (Function::Ceil, 1, 1),
            "sqrt" => (Function::Sqrt, 1, 1),
            "exists" => (Function::Exists, 1, 1),
            "len" => (Function::Len, 1, 1),
            _ => return None,
        })
    }

    /// Whether the environment must answer it (it needs the world or randomness).
    fn needs_env(self) -> bool {
        matches!(
            self,
            Function::Dist
                | Function::Sees
                | Function::Heard
                | Function::Count
                | Function::Nearest
                | Function::InState
                | Function::Time
                | Function::StateTime
                | Function::Random
                | Function::Chance
                | Function::Len
        )
    }
}

/// What an expression reads: the blackboards, the world and the agent's senses.
pub trait ExprEnv {
    /// A key's value: `name`, `group.name`, `global.name`, or `self`.
    fn lookup(&mut self, key: &str) -> Value;
    /// A field of a value (JSON data here; Lua tables need the environment).
    fn field(&mut self, value: &Value, name: &str) -> Value {
        value.json_field(name)
    }
    /// A function that needs the world, the senses or randomness.
    fn call(&mut self, function: Function, args: &[Value]) -> Value;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Concat,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Debug, Clone)]
enum Node {
    Lit(Value),
    /// `a.b.c`: a key (with its board prefix) and then fields.
    Path(Arc<str>, Vec<Arc<str>>),
    Call(Function, Vec<Node>),
    Neg(Box<Node>),
    Not(Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Bin(BinOp, Box<Node>, Box<Node>),
}

/// A parsed expression.
///
/// ```
/// use wgpu_cube::ai::{Expr, ExprEnv, Function, Value};
///
/// struct Board;
/// impl ExprEnv for Board {
///     fn lookup(&mut self, key: &str) -> Value {
///         match key {
///             "hunger" => Value::Num(0.8),
///             "group.alarm" => Value::Bool(false),
///             _ => Value::Nil,
///         }
///     }
///     fn call(&mut self, _: Function, _: &[Value]) -> Value {
///         Value::Nil
///     }
/// }
///
/// let expr = Expr::parse("hunger > 0.6 and not group.alarm").unwrap();
/// assert!(expr.eval(&mut Board).truthy());
/// assert_eq!(Expr::parse("missing or 3").unwrap().eval(&mut Board), Value::Int(3));
/// assert!(Expr::parse("hunger >").is_err());
/// ```
#[derive(Clone)]
pub struct Expr {
    source: Arc<str>,
    root: Arc<Node>,
}

impl fmt::Debug for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Expr({:?})", self.source)
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

impl Expr {
    pub fn parse(source: &str) -> Result<Self, String> {
        let tokens = tokenize(source)?;
        let mut parser = Parser { tokens, pos: 0 };
        let root = parser.or()?;
        if parser.pos < parser.tokens.len() {
            return Err(format!(
                "unexpected {} in {source:?}",
                parser.tokens[parser.pos].describe()
            ));
        }
        Ok(Self {
            source: Arc::from(source),
            root: Arc::new(root),
        })
    }

    /// An expression that is just a constant.
    pub fn constant(value: Value) -> Self {
        Self {
            source: Arc::from(value.to_string()),
            root: Arc::new(Node::Lit(value)),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// The blackboard keys it reads (for showing what a condition watches).
    pub fn keys(&self) -> Vec<String> {
        fn walk(node: &Node, out: &mut Vec<String>) {
            match node {
                Node::Path(key, _) => {
                    if !out.iter().any(|k| **k == **key) {
                        out.push(key.to_string());
                    }
                }
                Node::Call(_, args) => args.iter().for_each(|a| walk(a, out)),
                Node::Neg(a) | Node::Not(a) => walk(a, out),
                Node::And(a, b) | Node::Or(a, b) | Node::Bin(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Node::Lit(_) => {}
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    pub fn eval(&self, env: &mut dyn ExprEnv) -> Value {
        eval(&self.root, env)
    }
}

fn eval(node: &Node, env: &mut dyn ExprEnv) -> Value {
    match node {
        Node::Lit(value) => value.clone(),
        Node::Path(key, fields) => {
            let mut value = env.lookup(key);
            for field in fields {
                if value.is_nil() {
                    break;
                }
                value = env.field(&value, field);
            }
            value
        }
        Node::Call(function, args) => {
            let values: Vec<Value> = args.iter().map(|a| eval(a, env)).collect();
            if function.needs_env() {
                env.call(*function, &values)
            } else {
                pure_call(*function, &values)
            }
        }
        Node::Neg(a) => match eval(a, env) {
            Value::Int(i) => Value::Int(-i),
            Value::Num(n) => Value::Num(-n),
            Value::Vec3(v) => Value::Vec3(-v),
            _ => Value::Nil,
        },
        Node::Not(a) => Value::Bool(!eval(a, env).truthy()),
        Node::And(a, b) => {
            let left = eval(a, env);
            if left.truthy() {
                eval(b, env)
            } else {
                left
            }
        }
        Node::Or(a, b) => {
            let left = eval(a, env);
            if left.truthy() {
                left
            } else {
                eval(b, env)
            }
        }
        Node::Bin(op, a, b) => binary(*op, eval(a, env), eval(b, env)),
    }
}

fn pure_call(function: Function, args: &[Value]) -> Value {
    let num = |i: usize| args.get(i).and_then(Value::as_f64);
    match function {
        Function::Abs => num(0).map_or(Value::Nil, |x| Value::Num(x.abs())),
        Function::Floor => num(0).map_or(Value::Nil, |x| Value::Num(x.floor())),
        Function::Ceil => num(0).map_or(Value::Nil, |x| Value::Num(x.ceil())),
        Function::Sqrt => num(0).map_or(Value::Nil, |x| Value::Num(x.max(0.0).sqrt())),
        Function::Min | Function::Max => {
            let mut best: Option<f64> = None;
            for value in args {
                let Some(x) = value.as_f64() else {
                    return Value::Nil;
                };
                best = Some(match best {
                    None => x,
                    Some(b) if function == Function::Min => b.min(x),
                    Some(b) => b.max(x),
                });
            }
            best.map_or(Value::Nil, Value::Num)
        }
        Function::Clamp => match (num(0), num(1), num(2)) {
            (Some(x), Some(lo), Some(hi)) => Value::Num(x.max(lo).min(hi.max(lo))),
            _ => Value::Nil,
        },
        Function::Exists => Value::Bool(args.first().is_some_and(|v| !v.is_nil())),
        _ => Value::Nil,
    }
}

fn binary(op: BinOp, a: Value, b: Value) -> Value {
    use std::cmp::Ordering;
    let order = || -> Option<Ordering> {
        match (&a, &b) {
            (Value::Str(x), Value::Str(y)) => Some(x.cmp(y)),
            _ => a.as_f64()?.partial_cmp(&b.as_f64()?),
        }
    };
    match op {
        BinOp::Eq => Value::Bool(a == b),
        BinOp::Ne => Value::Bool(a != b),
        BinOp::Lt => Value::Bool(order() == Some(Ordering::Less)),
        BinOp::Le => Value::Bool(matches!(order(), Some(Ordering::Less | Ordering::Equal))),
        BinOp::Gt => Value::Bool(order() == Some(Ordering::Greater)),
        BinOp::Ge => Value::Bool(matches!(order(), Some(Ordering::Greater | Ordering::Equal))),
        BinOp::Concat => Value::str(format!("{a}{b}")),
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => arithmetic(op, &a, &b),
    }
}

fn arithmetic(op: BinOp, a: &Value, b: &Value) -> Value {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) if op != BinOp::Div => {
            let result = match op {
                BinOp::Add => x.checked_add(*y),
                BinOp::Sub => x.checked_sub(*y),
                BinOp::Mul => x.checked_mul(*y),
                BinOp::Rem if *y != 0 => Some(x.rem_euclid(*y)),
                _ => None,
            };
            result.map_or(Value::Nil, Value::Int)
        }
        (Value::Vec3(x), Value::Vec3(y)) => match op {
            BinOp::Add => Value::Vec3(*x + *y),
            BinOp::Sub => Value::Vec3(*x - *y),
            _ => Value::Nil,
        },
        (Value::Vec3(v), n) | (n, Value::Vec3(v)) if op == BinOp::Mul => match n.as_f64() {
            Some(n) => Value::Vec3(*v * n as f32),
            None => Value::Nil,
        },
        (Value::Vec3(v), n) if op == BinOp::Div => match n.as_f64() {
            Some(n) if n != 0.0 => Value::Vec3(*v / n as f32),
            _ => Value::Nil,
        },
        _ => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => match op {
                BinOp::Add => Value::Num(x + y),
                BinOp::Sub => Value::Num(x - y),
                BinOp::Mul => Value::Num(x * y),
                BinOp::Div if y != 0.0 => Value::Num(x / y),
                BinOp::Rem if y != 0.0 => Value::Num(x.rem_euclid(y)),
                _ => Value::Nil,
            },
            _ => Value::Nil,
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64, bool),
    Str(String),
    Ident(String),
    Op(&'static str),
}

impl Token {
    fn describe(&self) -> String {
        match self {
            Token::Num(n, _) => format!("number {n}"),
            Token::Str(s) => format!("text {s:?}"),
            Token::Ident(s) => format!("{s:?}"),
            Token::Op(op) => format!("{op:?}"),
        }
    }
}

const OPERATORS: [&str; 22] = [
    "==", "~=", "!=", "<=", ">=", "&&", "||", "..", "<", ">", "+", "-", "*", "/", "%", "(", ")",
    ",", ".", "!", "[", "]",
];

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = i;
            let mut float = false;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                // `1..2` is a concatenation, not a number.
                if chars[i] == '.' && chars.get(i + 1) == Some(&'.') {
                    break;
                }
                float |= chars[i] == '.';
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                float = true;
                i += 1;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = chars[start..i].iter().collect();
            let value = text
                .parse::<f64>()
                .map_err(|_| format!("bad number {text:?} in {source:?}"))?;
            tokens.push(Token::Num(value, float));
        } else if c == '\'' || c == '"' {
            let quote = c;
            i += 1;
            let mut text = String::new();
            loop {
                let Some(&ch) = chars.get(i) else {
                    return Err(format!("unfinished text in {source:?}"));
                };
                i += 1;
                if ch == quote {
                    break;
                }
                if ch == '\\' {
                    if let Some(&next) = chars.get(i) {
                        text.push(match next {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                        i += 1;
                    }
                } else {
                    text.push(ch);
                }
            }
            tokens.push(Token::Str(text));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect()));
        } else {
            let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
            let Some(op) = OPERATORS.iter().find(|op| rest.starts_with(**op)) else {
                return Err(format!("unexpected {c:?} in {source:?}"));
            };
            i += op.chars().count();
            tokens.push(Token::Op(op));
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Token::Op(o)) if *o == op)
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Ident(w)) if w == word)
    }

    fn expect_op(&mut self, op: &str) -> Result<(), String> {
        if self.is_op(op) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!(
                "expected {op:?}, found {}",
                self.peek().map_or("the end".into(), Token::describe)
            ))
        }
    }

    fn or(&mut self) -> Result<Node, String> {
        let mut left = self.and()?;
        while self.is_word("or") || self.is_op("||") {
            self.pos += 1;
            left = Node::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Node, String> {
        let mut left = self.not()?;
        while self.is_word("and") || self.is_op("&&") {
            self.pos += 1;
            left = Node::And(Box::new(left), Box::new(self.not()?));
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Node, String> {
        if self.is_word("not") || self.is_op("!") {
            self.pos += 1;
            return Ok(Node::Not(Box::new(self.not()?)));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Result<Node, String> {
        let left = self.concat()?;
        let op = match self.peek() {
            Some(Token::Op("==")) => BinOp::Eq,
            Some(Token::Op("~=" | "!=")) => BinOp::Ne,
            Some(Token::Op("<")) => BinOp::Lt,
            Some(Token::Op("<=")) => BinOp::Le,
            Some(Token::Op(">")) => BinOp::Gt,
            Some(Token::Op(">=")) => BinOp::Ge,
            _ => return Ok(left),
        };
        self.pos += 1;
        let right = self.concat()?;
        Ok(Node::Bin(op, Box::new(left), Box::new(right)))
    }

    fn concat(&mut self) -> Result<Node, String> {
        let left = self.sum()?;
        if self.is_op("..") {
            self.pos += 1;
            let right = self.concat()?;
            return Ok(Node::Bin(BinOp::Concat, Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    fn sum(&mut self) -> Result<Node, String> {
        let mut left = self.product()?;
        loop {
            let op = if self.is_op("+") {
                BinOp::Add
            } else if self.is_op("-") {
                BinOp::Sub
            } else {
                return Ok(left);
            };
            self.pos += 1;
            left = Node::Bin(op, Box::new(left), Box::new(self.product()?));
        }
    }

    fn product(&mut self) -> Result<Node, String> {
        let mut left = self.unary()?;
        loop {
            let op = if self.is_op("*") {
                BinOp::Mul
            } else if self.is_op("/") {
                BinOp::Div
            } else if self.is_op("%") {
                BinOp::Rem
            } else {
                return Ok(left);
            };
            self.pos += 1;
            left = Node::Bin(op, Box::new(left), Box::new(self.unary()?));
        }
    }

    fn unary(&mut self) -> Result<Node, String> {
        if self.is_op("-") {
            self.pos += 1;
            return Ok(Node::Neg(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Node, String> {
        let Some(token) = self.peek().cloned() else {
            return Err("the expression ends too soon".into());
        };
        self.pos += 1;
        match token {
            Token::Num(n, float) => Ok(Node::Lit(
                if !float && n.fract() == 0.0 && n.abs() < 9.0e15 {
                    Value::Int(n as i64)
                } else {
                    Value::Num(n)
                },
            )),
            Token::Str(s) => Ok(Node::Lit(Value::str(s))),
            Token::Op("(") => {
                let inner = self.or()?;
                self.expect_op(")")?;
                Ok(inner)
            }
            Token::Ident(word) => match word.as_str() {
                "true" => Ok(Node::Lit(Value::Bool(true))),
                "false" => Ok(Node::Lit(Value::Bool(false))),
                "nil" => Ok(Node::Lit(Value::Nil)),
                "and" | "or" | "not" => Err(format!("unexpected {word:?}")),
                _ if self.is_op("(") => self.call(&word),
                _ => self.path(word),
            },
            other => Err(format!("unexpected {}", other.describe())),
        }
    }

    fn call(&mut self, name: &str) -> Result<Node, String> {
        let Some((function, min, max)) = Function::from_name(name) else {
            return Err(format!("unknown function {name:?}"));
        };
        self.expect_op("(")?;
        let mut args = Vec::new();
        if !self.is_op(")") {
            loop {
                args.push(self.or()?);
                if self.is_op(",") {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        self.expect_op(")")?;
        if args.len() < min || args.len() > max {
            return Err(format!(
                "{name}() takes {} arguments, not {}",
                if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                },
                args.len()
            ));
        }
        Ok(Node::Call(function, args))
    }

    fn path(&mut self, first: String) -> Result<Node, String> {
        let mut segments = vec![first];
        while self.is_op(".") || self.is_op("[") {
            if self.is_op("[") {
                // `list[2]` reads the second item.
                self.pos += 1;
                let Some(Token::Num(index, _)) = self.peek().cloned() else {
                    return Err("expected a number in [ ]".into());
                };
                self.pos += 1;
                self.expect_op("]")?;
                segments.push(format!("{}", index as i64));
                continue;
            }
            self.pos += 1;
            match self.peek().cloned() {
                Some(Token::Ident(name)) => {
                    self.pos += 1;
                    segments.push(name);
                }
                other => {
                    return Err(format!(
                        "expected a name after '.', found {}",
                        other.map_or("the end".into(), |t| t.describe())
                    ))
                }
            }
        }
        // `group.x` and `global.x` are keys of those boards.
        let key = if segments.len() > 1 && (segments[0] == "group" || segments[0] == "global") {
            let board = segments.remove(0);
            format!("{board}.{}", segments.remove(0))
        } else {
            segments.remove(0)
        };
        Ok(Node::Path(
            Arc::from(key),
            segments.into_iter().map(Arc::from).collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Env {
        values: HashMap<String, Value>,
        calls: Vec<Function>,
    }

    impl ExprEnv for Env {
        fn lookup(&mut self, key: &str) -> Value {
            self.values.get(key).cloned().unwrap_or_default()
        }
        fn call(&mut self, function: Function, args: &[Value]) -> Value {
            self.calls.push(function);
            match function {
                Function::Dist => Value::Num(args.len() as f64),
                Function::Sees => {
                    Value::Bool(args.first().and_then(Value::as_str) == Some("thief"))
                }
                _ => Value::Nil,
            }
        }
    }

    fn eval(source: &str, env: &mut Env) -> Value {
        Expr::parse(source)
            .unwrap_or_else(|e| panic!("{source}: {e}"))
            .eval(env)
    }

    #[test]
    fn arithmetic_and_precedence() {
        let mut env = Env::default();
        assert_eq!(eval("1 + 2 * 3", &mut env), Value::Int(7));
        assert_eq!(eval("(1 + 2) * 3", &mut env), Value::Int(9));
        assert_eq!(eval("7 / 2", &mut env), Value::Num(3.5));
        assert_eq!(eval("-2 + 5", &mut env), Value::Int(3));
        assert_eq!(eval("10 % 4", &mut env), Value::Int(2));
        assert_eq!(eval("1.5e1", &mut env), Value::Num(15.0));
        assert_eq!(eval("max(1, 4, 2)", &mut env), Value::Num(4.0));
        assert_eq!(eval("clamp(5, 0, 1)", &mut env), Value::Num(1.0));
        assert_eq!(eval("'a' .. 1 .. 'b'", &mut env), Value::str("a1b"));
    }

    #[test]
    fn logic_works_like_lua() {
        let mut env = Env::default();
        env.values.insert("hunger".into(), Value::Num(0.7));
        env.values.insert("group.alarm".into(), Value::Bool(true));
        assert!(eval("hunger > 0.5 and group.alarm", &mut env).truthy());
        assert!(!eval("not group.alarm", &mut env).truthy());
        assert_eq!(eval("missing or 'x'", &mut env), Value::str("x"));
        assert_eq!(eval("missing and 1", &mut env), Value::Nil);
        assert!(eval("missing == nil", &mut env).truthy());
        assert!(eval("hunger ~= 1 && hunger != 2", &mut env).truthy());
        assert!(!eval("missing < 3", &mut env).truthy());
        assert_eq!(eval("missing + 1", &mut env), Value::Nil);
    }

    #[test]
    fn fields_and_functions() {
        let mut env = Env::default();
        env.values.insert(
            "spot".into(),
            Value::from_json(
                &serde_json::json!({"kind": "stall", "pos": [1, 2, 3], "list": [4, 5]}),
            ),
        );
        assert_eq!(eval("spot.kind", &mut env), Value::str("stall"));
        assert_eq!(eval("spot.pos.y", &mut env), Value::Num(2.0));
        assert_eq!(eval("spot.list[2]", &mut env), Value::Int(5));
        assert_eq!(eval("dist(spot, spot)", &mut env), Value::Num(2.0));
        assert!(eval("sees('thief')", &mut env).truthy());
        assert_eq!(env.calls, vec![Function::Dist, Function::Sees]);
        assert_eq!(
            Expr::parse("hunger > 1 and group.alarm and spot.kind")
                .unwrap()
                .keys(),
            vec!["hunger", "group.alarm", "spot"]
        );
    }

    #[test]
    fn errors_name_the_problem() {
        assert!(Expr::parse("frobnicate(1)")
            .unwrap_err()
            .contains("unknown function"));
        assert!(Expr::parse("abs(1, 2)").unwrap_err().contains("takes 1"));
        assert!(Expr::parse("(1 + 2")
            .unwrap_err()
            .contains("expected \")\""));
        assert!(Expr::parse("'open").is_err());
        assert!(Expr::parse("1 $ 2").is_err());
    }
}
