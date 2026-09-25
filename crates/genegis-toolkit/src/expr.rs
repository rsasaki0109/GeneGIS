//! Small SQL-like expression language for attribute filters and field
//! calculation.
//!
//! ```text
//! 人口 >= 1000 AND 区名 LIKE '%中%'
//! "pop density" > 5000 OR NOT (type IN ('a', 'b'))
//! population / area_km2
//! name IS NOT NULL
//! ```
//!
//! Field names may be bare (including Japanese), double-quoted, or
//! bracketed. Strings use single quotes. Arithmetic on NULL yields NULL and
//! comparisons with NULL are false, as in SQL.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::{Result, ToolkitError};

/// Parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Literal value.
    Literal(Value),
    /// Field reference.
    Field(String),
    /// Unary operator.
    Unary(UnaryOp, Box<Expr>),
    /// Binary operator.
    Binary(Box<Expr>, BinaryOp, Box<Expr>),
    /// `x IN (a, b)` / `x NOT IN (…)`.
    In(Box<Expr>, Vec<Expr>, bool),
    /// `x IS NULL` / `x IS NOT NULL`.
    IsNull(Box<Expr>, bool),
    /// `x LIKE 'pattern'` / `x NOT LIKE …` with `%` and `_` wildcards.
    Like(Box<Expr>, Box<Expr>, bool),
    /// Function call.
    Call(String, Vec<Expr>),
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    /// Logical NOT.
    Not,
    /// Arithmetic negation.
    Neg,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOp {
    /// `OR`
    Or,
    /// `AND`
    And,
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
}

fn err(reason: impl Into<String>) -> ToolkitError {
    ToolkitError::Expression(reason.into())
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Quoted(String),
    Op(&'static str),
    Open,
    Close,
    Comma,
}

fn tokenize(text: &str) -> Result<Vec<Tok>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let op2 = ["<=", ">=", "!=", "<>", "==", "&&", "||"]
            .into_iter()
            .find(|op| *op == two);
        if let Some(op) = op2 {
            out.push(Tok::Op(match op {
                "<>" => "!=",
                "==" => "=",
                "&&" => "AND",
                "||" => "OR",
                other => other,
            }));
            i += 2;
            continue;
        }
        match c {
            '(' | '（' => {
                out.push(Tok::Open);
                i += 1;
            }
            ')' | '）' => {
                out.push(Tok::Close);
                i += 1;
            }
            ',' | '、' => {
                out.push(Tok::Comma);
                i += 1;
            }
            '=' | '<' | '>' | '+' | '-' | '*' | '/' | '%' | '!' => {
                out.push(Tok::Op(match c {
                    '=' => "=",
                    '<' => "<",
                    '>' => ">",
                    '+' => "+",
                    '-' => "-",
                    '*' => "*",
                    '/' => "/",
                    '%' => "%",
                    _ => "NOT",
                }));
                i += 1;
            }
            '\'' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(err("unterminated string literal")),
                        Some('\'') if chars.get(i + 1) == Some(&'\'') => {
                            s.push('\'');
                            i += 2;
                        }
                        Some('\'') => {
                            i += 1;
                            break;
                        }
                        Some(ch) => {
                            s.push(*ch);
                            i += 1;
                        }
                    }
                }
                out.push(Tok::Str(s));
            }
            '"' | '[' | '`' => {
                let close = match c {
                    '"' => '"',
                    '[' => ']',
                    _ => '`',
                };
                let start = i + 1;
                let end = chars[start..]
                    .iter()
                    .position(|ch| *ch == close)
                    .ok_or_else(|| err("unterminated quoted field name"))?;
                out.push(Tok::Quoted(chars[start..start + end].iter().collect()));
                i = start + end + 1;
            }
            c if c.is_ascii_digit()
                || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) =>
            {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_digit()
                        || chars[i] == '.'
                        || matches!(chars[i], 'e' | 'E')
                        || (matches!(chars[i], '+' | '-') && matches!(chars[i - 1], 'e' | 'E')))
                {
                    i += 1;
                }
                let literal: String = chars[start..i].iter().collect();
                out.push(Tok::Num(
                    literal
                        .parse()
                        .map_err(|_| err(format!("invalid number {literal}")))?,
                ));
            }
            _ => {
                let start = i;
                while i < chars.len()
                    && !chars[i].is_whitespace()
                    && !"()（）,、=<>!+-*/%'\"[`".contains(chars[i])
                {
                    i += 1;
                }
                out.push(Tok::Ident(chars[start..i].iter().collect()));
            }
        }
    }
    Ok(out)
}

/// Parse an expression.
pub fn parse(text: &str) -> Result<Expr> {
    let tokens = tokenize(text)?;
    if tokens.is_empty() {
        return Err(err("expression is empty"));
    }
    let mut parser = Parser { tokens, pos: 0 };
    let expr = parser.or()?;
    if parser.pos != parser.tokens.len() {
        return Err(err(format!(
            "unexpected token {:?}",
            parser.tokens[parser.pos]
        )));
    }
    Ok(expr)
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn keyword(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(w)) if w.eq_ignore_ascii_case(word))
            || matches!(self.peek(), Some(Tok::Op(op)) if *op == word)
    }

    fn keyword_at(&self, offset: usize, word: &str) -> bool {
        matches!(self.tokens.get(self.pos + offset), Some(Tok::Ident(w)) if w.eq_ignore_ascii_case(word))
            || matches!(self.tokens.get(self.pos + offset), Some(Tok::Op(op)) if *op == word)
    }

    fn or(&mut self) -> Result<Expr> {
        let mut left = self.and()?;
        while self.keyword("OR") || self.keyword("または") {
            self.pos += 1;
            left = Expr::Binary(Box::new(left), BinaryOp::Or, Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr> {
        let mut left = self.not()?;
        while self.keyword("AND") || self.keyword("かつ") {
            self.pos += 1;
            left = Expr::Binary(Box::new(left), BinaryOp::And, Box::new(self.not()?));
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Expr> {
        if self.keyword("NOT") {
            self.pos += 1;
            return Ok(Expr::Unary(UnaryOp::Not, Box::new(self.not()?)));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Result<Expr> {
        let left = self.additive()?;
        if self.keyword("IS") {
            self.pos += 1;
            let negated = if self.keyword("NOT") {
                self.pos += 1;
                true
            } else {
                false
            };
            if !self.keyword("NULL") {
                return Err(err("expected NULL after IS"));
            }
            self.pos += 1;
            return Ok(Expr::IsNull(Box::new(left), negated));
        }
        let negated =
            self.keyword("NOT") && (self.keyword_at(1, "IN") || self.keyword_at(1, "LIKE"));
        if negated {
            self.pos += 1;
        }
        if self.keyword("IN") {
            self.pos += 1;
            if self.peek() != Some(&Tok::Open) {
                return Err(err("expected ( after IN"));
            }
            self.pos += 1;
            let mut items = vec![self.additive()?];
            while self.peek() == Some(&Tok::Comma) {
                self.pos += 1;
                items.push(self.additive()?);
            }
            if self.peek() != Some(&Tok::Close) {
                return Err(err("expected ) to close IN list"));
            }
            self.pos += 1;
            return Ok(Expr::In(Box::new(left), items, negated));
        }
        if self.keyword("LIKE") {
            self.pos += 1;
            return Ok(Expr::Like(
                Box::new(left),
                Box::new(self.additive()?),
                negated,
            ));
        }
        if negated {
            return Err(err("expected IN or LIKE after NOT"));
        }
        let op = match self.peek() {
            Some(Tok::Op("=")) => BinaryOp::Eq,
            Some(Tok::Op("!=")) => BinaryOp::Ne,
            Some(Tok::Op("<")) => BinaryOp::Lt,
            Some(Tok::Op("<=")) => BinaryOp::Le,
            Some(Tok::Op(">")) => BinaryOp::Gt,
            Some(Tok::Op(">=")) => BinaryOp::Ge,
            _ => return Ok(left),
        };
        self.pos += 1;
        Ok(Expr::Binary(Box::new(left), op, Box::new(self.additive()?)))
    }

    fn additive(&mut self) -> Result<Expr> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op("+")) => BinaryOp::Add,
                Some(Tok::Op("-")) => BinaryOp::Sub,
                _ => return Ok(left),
            };
            self.pos += 1;
            left = Expr::Binary(Box::new(left), op, Box::new(self.multiplicative()?));
        }
    }

    fn multiplicative(&mut self) -> Result<Expr> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op("*")) => BinaryOp::Mul,
                Some(Tok::Op("/")) => BinaryOp::Div,
                Some(Tok::Op("%")) => BinaryOp::Rem,
                _ => return Ok(left),
            };
            self.pos += 1;
            left = Expr::Binary(Box::new(left), op, Box::new(self.unary()?));
        }
    }

    fn unary(&mut self) -> Result<Expr> {
        if self.peek() == Some(&Tok::Op("-")) {
            self.pos += 1;
            return Ok(Expr::Unary(UnaryOp::Neg, Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr> {
        let token = self
            .tokens
            .get(self.pos)
            .cloned()
            .ok_or_else(|| err("unexpected end of expression"))?;
        self.pos += 1;
        match token {
            Tok::Num(n) => Ok(Expr::Literal(number(n))),
            Tok::Str(s) => Ok(Expr::Literal(Value::from(s))),
            Tok::Quoted(name) => Ok(Expr::Field(name)),
            Tok::Open => {
                let inner = self.or()?;
                if self.peek() != Some(&Tok::Close) {
                    return Err(err("missing closing parenthesis"));
                }
                self.pos += 1;
                Ok(inner)
            }
            Tok::Ident(word) => {
                match word.to_ascii_uppercase().as_str() {
                    "TRUE" => return Ok(Expr::Literal(Value::Bool(true))),
                    "FALSE" => return Ok(Expr::Literal(Value::Bool(false))),
                    "NULL" => return Ok(Expr::Literal(Value::Null)),
                    _ => {}
                }
                if self.peek() == Some(&Tok::Open) {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::Close) {
                        args.push(self.or()?);
                        while self.peek() == Some(&Tok::Comma) {
                            self.pos += 1;
                            args.push(self.or()?);
                        }
                    }
                    if self.peek() != Some(&Tok::Close) {
                        return Err(err(format!("missing ) after arguments to {word}")));
                    }
                    self.pos += 1;
                    return Ok(Expr::Call(word.to_ascii_lowercase(), args));
                }
                Ok(Expr::Field(word))
            }
            other => Err(err(format!("unexpected token {other:?}"))),
        }
    }
}

fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    if a.is_null() || b.is_null() {
        return None;
    }
    if let (Some(x), Some(y)) = (as_f64(a), as_f64(b)) {
        if !(a.is_string() && b.is_string()) {
            return x.partial_cmp(&y);
        }
    }
    match (a, b) {
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => Some(a.to_string().cmp(&b.to_string())),
    }
}

fn like(text: &str, pattern: &str) -> bool {
    fn go(t: &[char], p: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (None, Some(_)) => false,
            (Some('%'), _) => go(t, &p[1..]) || (!t.is_empty() && go(&t[1..], p)),
            (Some('_'), Some(_)) => go(&t[1..], &p[1..]),
            (Some(pc), Some(tc)) if pc == tc => go(&t[1..], &p[1..]),
            _ => false,
        }
    }
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    go(&t, &p)
}

impl Expr {
    /// Field names referenced by the expression.
    pub fn fields(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_fields(&mut out);
        out.sort();
        out.dedup();
        out
    }

    fn collect_fields(&self, out: &mut Vec<String>) {
        match self {
            Expr::Literal(_) => {}
            Expr::Field(name) => out.push(name.clone()),
            Expr::Unary(_, e) | Expr::IsNull(e, _) => e.collect_fields(out),
            Expr::Binary(a, _, b) | Expr::Like(a, b, _) => {
                a.collect_fields(out);
                b.collect_fields(out);
            }
            Expr::In(e, items, _) => {
                e.collect_fields(out);
                for item in items {
                    item.collect_fields(out);
                }
            }
            Expr::Call(_, args) => {
                for arg in args {
                    arg.collect_fields(out);
                }
            }
        }
    }

    /// Evaluate against one feature's properties.
    pub fn eval(&self, properties: &BTreeMap<String, Value>) -> Result<Value> {
        Ok(match self {
            Expr::Literal(v) => v.clone(),
            Expr::Field(name) => properties
                .get(name)
                .cloned()
                .ok_or_else(|| err(format!("unknown field {name}")))?,
            Expr::Unary(UnaryOp::Not, e) => Value::Bool(!truthy(&e.eval(properties)?)),
            Expr::Unary(UnaryOp::Neg, e) => match as_f64(&e.eval(properties)?) {
                Some(v) => number(-v),
                None => Value::Null,
            },
            Expr::Binary(a, op, b) => {
                let left = a.eval(properties)?;
                match op {
                    BinaryOp::And => {
                        if !truthy(&left) {
                            return Ok(Value::Bool(false));
                        }
                        Value::Bool(truthy(&b.eval(properties)?))
                    }
                    BinaryOp::Or => {
                        if truthy(&left) {
                            return Ok(Value::Bool(true));
                        }
                        Value::Bool(truthy(&b.eval(properties)?))
                    }
                    BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge => {
                        let right = b.eval(properties)?;
                        let Some(ordering) = compare(&left, &right) else {
                            return Ok(Value::Bool(false));
                        };
                        use std::cmp::Ordering::*;
                        Value::Bool(match op {
                            BinaryOp::Eq => ordering == Equal,
                            BinaryOp::Ne => ordering != Equal,
                            BinaryOp::Lt => ordering == Less,
                            BinaryOp::Le => ordering != Greater,
                            BinaryOp::Gt => ordering == Greater,
                            _ => ordering != Less,
                        })
                    }
                    BinaryOp::Add if left.is_string() => {
                        let right = b.eval(properties)?;
                        Value::from(format!(
                            "{}{}",
                            left.as_str().unwrap_or_default(),
                            right
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| right.to_string())
                        ))
                    }
                    _ => {
                        let right = b.eval(properties)?;
                        match (as_f64(&left), as_f64(&right)) {
                            (Some(x), Some(y)) => {
                                let v = match op {
                                    BinaryOp::Add => x + y,
                                    BinaryOp::Sub => x - y,
                                    BinaryOp::Mul => x * y,
                                    BinaryOp::Div if y == 0.0 => return Ok(Value::Null),
                                    BinaryOp::Div => x / y,
                                    BinaryOp::Rem if y == 0.0 => return Ok(Value::Null),
                                    _ => x % y,
                                };
                                if matches!(op, BinaryOp::Div) {
                                    serde_json::Number::from_f64(v)
                                        .map(Value::Number)
                                        .unwrap_or(Value::Null)
                                } else {
                                    number(v)
                                }
                            }
                            _ => Value::Null,
                        }
                    }
                }
            }
            Expr::In(e, items, negated) => {
                let value = e.eval(properties)?;
                if value.is_null() {
                    return Ok(Value::Bool(false));
                }
                let mut found = false;
                for item in items {
                    if compare(&value, &item.eval(properties)?) == Some(std::cmp::Ordering::Equal) {
                        found = true;
                        break;
                    }
                }
                Value::Bool(found != *negated)
            }
            Expr::IsNull(e, negated) => Value::Bool(e.eval(properties)?.is_null() != *negated),
            Expr::Like(e, pattern, negated) => {
                let value = e.eval(properties)?;
                let pattern = pattern.eval(properties)?;
                match (value, pattern) {
                    (Value::Null, _) | (_, Value::Null) => Value::Bool(false),
                    (v, Value::String(p)) => {
                        let text = v
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string());
                        Value::Bool(like(&text, &p) != *negated)
                    }
                    _ => return Err(err("LIKE needs a string pattern")),
                }
            }
            Expr::Call(name, args) => {
                let values = args
                    .iter()
                    .map(|a| a.eval(properties))
                    .collect::<Result<Vec<_>>>()?;
                call(name, &values)?
            }
        })
    }
}

fn call(name: &str, args: &[Value]) -> Result<Value> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);
    let num = |i: usize| as_f64(&arg(i));
    let float = |v: Option<f64>| {
        v.and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    };
    Ok(match name {
        "abs" => float(num(0).map(f64::abs)),
        "round" => {
            let digits = num(1).unwrap_or(0.0) as i32;
            let factor = 10f64.powi(digits);
            let v = num(0).map(|v| (v * factor).round() / factor);
            if digits <= 0 {
                v.map(number).unwrap_or(Value::Null)
            } else {
                float(v)
            }
        }
        "floor" => num(0).map(|v| number(v.floor())).unwrap_or(Value::Null),
        "ceil" => num(0).map(|v| number(v.ceil())).unwrap_or(Value::Null),
        "sqrt" => float(num(0).filter(|v| *v >= 0.0).map(f64::sqrt)),
        "lower" => arg(0)
            .as_str()
            .map(|s| Value::from(s.to_lowercase()))
            .unwrap_or(Value::Null),
        "upper" => arg(0)
            .as_str()
            .map(|s| Value::from(s.to_uppercase()))
            .unwrap_or(Value::Null),
        "length" | "len" => arg(0)
            .as_str()
            .map(|s| Value::from(s.chars().count() as i64))
            .unwrap_or(Value::Null),
        "coalesce" => args
            .iter()
            .find(|v| !v.is_null())
            .cloned()
            .unwrap_or(Value::Null),
        "number" | "to_number" => float(num(0)),
        "text" | "to_text" => match arg(0) {
            Value::Null => Value::Null,
            Value::String(s) => Value::from(s),
            other => Value::from(other.to_string()),
        },
        other => return Err(err(format!("unknown function {other}"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props() -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("人口".to_string(), Value::from(1500)),
            ("区名".to_string(), Value::from("中区")),
            ("area km2".to_string(), Value::from(9.38)),
            ("code".to_string(), Value::from("23106")),
            ("note".to_string(), Value::Null),
        ])
    }

    fn eval(text: &str) -> Value {
        parse(text).unwrap().eval(&props()).unwrap()
    }

    #[test]
    fn filters_with_japanese_fields_and_like() {
        assert_eq!(eval("人口 >= 1000 AND 区名 LIKE '%中%'"), Value::Bool(true));
        assert_eq!(eval("人口 < 1000 OR 区名 = '東区'"), Value::Bool(false));
        assert_eq!(eval("区名 IN ('中区', '東区')"), Value::Bool(true));
        assert_eq!(eval("区名 NOT IN ('中区')"), Value::Bool(false));
        assert_eq!(eval("note IS NULL AND code IS NOT NULL"), Value::Bool(true));
        assert_eq!(eval("NOT (人口 = 1500)"), Value::Bool(false));
    }

    #[test]
    fn computes_arithmetic_and_functions() {
        assert_eq!(
            eval("round(人口 / \"area km2\", 1)"),
            serde_json::json!(159.9)
        );
        assert_eq!(eval("人口 * 2 - 1"), Value::from(2999));
        assert_eq!(eval("人口 / 0"), Value::Null);
        assert_eq!(eval("length(区名)"), Value::from(2));
        assert_eq!(eval("code = 23106"), Value::Bool(true));
    }

    #[test]
    fn null_comparisons_are_false_and_unknown_fields_fail() {
        assert_eq!(eval("note > 1"), Value::Bool(false));
        assert!(parse("missing > 1").unwrap().eval(&props()).is_err());
        assert!(parse("人口 >").is_err());
        assert!(parse("(人口 > 1").is_err());
        assert_eq!(
            parse("人口 > 1 AND 区名 = 'a'").unwrap().fields(),
            vec!["人口", "区名"]
        );
    }
}
