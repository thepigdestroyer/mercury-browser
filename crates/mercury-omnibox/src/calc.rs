// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The calculator's parser and evaluator: a hand-written recursive-descent parser over `f64`, with
//! no `eval` and no dependency (spec D3).

use std::f64::consts;

/// The longest input read, in bytes.
const MAX_BYTES: usize = 256;

/// The deepest nesting of parentheses, function calls and prefix signs.
const MAX_DEPTH: usize = 32;

/// The value of `input` when it is arithmetic worth an answer row: it holds at least one
/// operator, function call or percent sign (a bare number or constant is not an answer).
pub(crate) fn evaluate(input: &str) -> Option<f64> {
    let expr = parse(input)?;
    if is_bare(&expr) {
        return None;
    }
    finite(eval(&expr))
}

/// The value of `input` as any expression, including a bare number; for the unit converter's
/// quantity (`2 * 3` in `2 * 3 km to m`).
pub(crate) fn value_of(input: &str) -> Option<f64> {
    finite(eval(&parse(input)?))
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

enum Expr {
    Literal(f64),
    Negate(Box<Expr>),
    Percent(Box<Expr>),
    Call(fn(f64) -> f64, Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
}

impl Op {
    fn apply(self, left: f64, right: f64) -> f64 {
        match self {
            Op::Add => left + right,
            Op::Sub => left - right,
            Op::Mul => left * right,
            Op::Div => left / right,
            Op::Rem => left % right,
            Op::Pow => left.powf(right),
        }
    }
}

#[derive(Clone, PartialEq)]
enum Token {
    Number(f64),
    Word(String),
    Symbol(char),
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    if input.len() > MAX_BYTES {
        return None;
    }
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while let Some(&c) = chars.get(at) {
        if c.is_ascii_whitespace() {
            at += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(at + 1).is_some_and(char::is_ascii_digit))
        {
            let (value, next) = number(&chars, at)?;
            tokens.push(Token::Number(value));
            at = next;
        } else if c.is_ascii_alphabetic() {
            let start = at;
            while chars.get(at).is_some_and(char::is_ascii_alphanumeric) {
                at += 1;
            }
            let word = slice(&chars, start, at).to_ascii_lowercase();
            tokens.push(Token::Word(word));
        } else {
            let symbol = match c {
                '×' => '*',
                '÷' => '/',
                '*' if chars.get(at + 1) == Some(&'*') => {
                    at += 1;
                    '^'
                }
                '+' | '-' | '*' | '/' | '^' | '%' | '(' | ')' => c,
                _ => return None,
            };
            tokens.push(Token::Symbol(symbol));
            at += 1;
        }
    }
    Some(tokens)
}

fn slice(chars: &[char], start: usize, end: usize) -> String {
    chars.get(start..end).unwrap_or_default().iter().collect()
}

fn number(chars: &[char], start: usize) -> Option<(f64, usize)> {
    let mut at = start;
    let mut text = String::new();
    take_digits(chars, &mut at, &mut text);
    if chars.get(at) == Some(&'.') && chars.get(at + 1).is_some_and(char::is_ascii_digit) {
        text.push('.');
        at += 1;
        take_digits(chars, &mut at, &mut text);
    }
    if matches!(chars.get(at).copied(), Some('e' | 'E')) {
        let mut digits_at = at + 1;
        let sign = chars
            .get(digits_at)
            .copied()
            .filter(|c| matches!(*c, '+' | '-'));
        if sign.is_some() {
            digits_at += 1;
        }
        if chars.get(digits_at).is_some_and(char::is_ascii_digit) {
            text.push('e');
            if let Some(sign) = sign {
                text.push(sign);
            }
            at = digits_at;
            take_digits(chars, &mut at, &mut text);
        }
    }
    let value = text.parse::<f64>().ok()?;
    Some((value, at))
}

/// Reads digits, where a single `_` may join two digits (`1_000`).
fn take_digits(chars: &[char], at: &mut usize, text: &mut String) {
    let mut after_digit = false;
    while let Some(&c) = chars.get(*at) {
        if c.is_ascii_digit() {
            text.push(c);
            after_digit = true;
        } else if c == '_' && after_digit && chars.get(*at + 1).is_some_and(char::is_ascii_digit) {
            after_digit = false;
        } else {
            break;
        }
        *at += 1;
    }
}

fn parse(input: &str) -> Option<Expr> {
    let mut parser = Parser {
        tokens: tokenize(input)?,
        at: 0,
        depth: 0,
    };
    let expr = parser.additive()?;
    (parser.at == parser.tokens.len()).then_some(expr)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn eat(&mut self, symbol: char) -> bool {
        let found = self.peek() == Some(&Token::Symbol(symbol));
        if found {
            self.at += 1;
        }
        found
    }

    fn nest(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_DEPTH).then_some(())
    }

    fn unnest(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn additive(&mut self) -> Option<Expr> {
        let mut left = self.multiplicative()?;
        loop {
            let op = if self.eat('+') {
                Op::Add
            } else if self.eat('-') {
                Op::Sub
            } else {
                return Some(left);
            };
            let right = self.multiplicative()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn multiplicative(&mut self) -> Option<Expr> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Symbol('*')) => Op::Mul,
                Some(Token::Symbol('/')) => Op::Div,
                Some(Token::Word(word)) if word == "x" => Op::Mul,
                Some(Token::Word(word)) if word == "mod" => Op::Rem,
                Some(Token::Word(word)) if word == "of" && matches!(left, Expr::Percent(_)) => {
                    Op::Mul
                }
                _ => return Some(left),
            };
            self.at += 1;
            let right = self.unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> Option<Expr> {
        let negative = self.eat('-');
        if !negative && !self.eat('+') {
            return self.power();
        }
        self.nest()?;
        let inner = self.unary()?;
        self.unnest();
        Some(if negative {
            Expr::Negate(Box::new(inner))
        } else {
            inner
        })
    }

    fn power(&mut self) -> Option<Expr> {
        let base = self.postfix()?;
        if !self.eat('^') {
            return Some(base);
        }
        self.nest()?;
        let exponent = self.unary()?;
        self.unnest();
        Some(Expr::Binary(Op::Pow, Box::new(base), Box::new(exponent)))
    }

    fn postfix(&mut self) -> Option<Expr> {
        let atom = self.atom()?;
        Some(if self.eat('%') {
            Expr::Percent(Box::new(atom))
        } else {
            atom
        })
    }

    fn atom(&mut self) -> Option<Expr> {
        let token = self.peek()?.clone();
        self.at += 1;
        match token {
            Token::Number(value) => Some(Expr::Literal(value)),
            Token::Symbol('(') => {
                self.nest()?;
                let inner = self.additive()?;
                if !self.eat(')') {
                    return None;
                }
                self.unnest();
                Some(inner)
            }
            Token::Word(word) => self.word(&word),
            Token::Symbol(_) => None,
        }
    }

    fn word(&mut self, word: &str) -> Option<Expr> {
        if let Some(value) = constant(word) {
            return Some(Expr::Literal(value));
        }
        let function = function(word)?;
        if !self.eat('(') {
            return None;
        }
        self.nest()?;
        let argument = self.additive()?;
        if !self.eat(')') {
            return None;
        }
        self.unnest();
        Some(Expr::Call(function, Box::new(argument)))
    }
}

fn constant(word: &str) -> Option<f64> {
    match word {
        "pi" => Some(consts::PI),
        "e" => Some(consts::E),
        "tau" => Some(consts::TAU),
        _ => None,
    }
}

fn function(word: &str) -> Option<fn(f64) -> f64> {
    let function: fn(f64) -> f64 = match word {
        "sqrt" => f64::sqrt,
        "cbrt" => f64::cbrt,
        "abs" => f64::abs,
        "ln" => f64::ln,
        "log" => f64::log10,
        "log2" => f64::log2,
        "exp" => f64::exp,
        "sin" => f64::sin,
        "cos" => f64::cos,
        "tan" => f64::tan,
        "asin" => f64::asin,
        "acos" => f64::acos,
        "atan" => f64::atan,
        "floor" => f64::floor,
        "ceil" => f64::ceil,
        "round" => f64::round,
        _ => return None,
    };
    Some(function)
}

/// A number, or a sign on a number: nothing to calculate.
fn is_bare(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_) => true,
        Expr::Negate(inner) => is_bare(inner),
        _ => false,
    }
}

fn eval(expr: &Expr) -> f64 {
    match expr {
        Expr::Literal(value) => *value,
        Expr::Negate(inner) => -eval(inner),
        Expr::Percent(inner) => eval(inner) / 100.0,
        Expr::Call(function, argument) => function(eval(argument)),
        Expr::Binary(op, left, right) => {
            let a = eval(left);
            match (op, &**right) {
                (Op::Add, Expr::Percent(percent)) => a * (1.0 + eval(percent) / 100.0),
                (Op::Sub, Expr::Percent(percent)) => a * (1.0 - eval(percent) / 100.0),
                _ => op.apply(a, eval(right)),
            }
        }
    }
}
