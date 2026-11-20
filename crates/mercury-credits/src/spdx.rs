// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! SPDX licence expressions as Cargo writes them, and the rule that decides which of their
//! licences Mercury credits.
//!
//! The grammar is SPDX's: `OR` binds loosest, then `AND`, then `WITH` (an exception, glued to the
//! licence before it), with parentheses. Operators are accepted in any case, and the old `/`
//! that many crates still use means `OR`. `Apache-2.0 WITH LLVM-exception` is one term.
//!
//! A term is *taken* when Mercury may use the crate under it: it is in the allow-list of
//! `deny.toml` or in the crate's own exception list. For `A OR B` every taken branch is
//! credited (a superset is harmless; the allow-list is the one policy), and none taken is a
//! problem. For `A AND B` every term must be taken.
use std::collections::{BTreeMap, BTreeSet};

/// One licence, with the exception that modifies it, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub id: String,
    pub exception: Option<String>,
}

impl Term {
    pub fn plain(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            exception: None,
        }
    }

    /// `Apache-2.0 WITH LLVM-exception`, or just the id.
    pub fn key(&self) -> String {
        match &self.exception {
            Some(exception) => format!("{} WITH {exception}", self.id),
            None => self.id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Term(Term),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid licence expression {expression:?}: {reason}")]
pub struct ParseError {
    pub expression: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Open,
    Close,
    Slash,
    Word(String),
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let flush = |word: &mut String, tokens: &mut Vec<Token>| {
        if !word.is_empty() {
            tokens.push(Token::Word(std::mem::take(word)));
        }
    };
    for c in text.chars() {
        match c {
            '(' => {
                flush(&mut word, &mut tokens);
                tokens.push(Token::Open);
            }
            ')' => {
                flush(&mut word, &mut tokens);
                tokens.push(Token::Close);
            }
            '/' => {
                flush(&mut word, &mut tokens);
                tokens.push(Token::Slash);
            }
            c if c.is_whitespace() => flush(&mut word, &mut tokens),
            c => word.push(c),
        }
    }
    flush(&mut word, &mut tokens);
    tokens
}

fn is_operator(word: &str) -> bool {
    ["AND", "OR", "WITH"]
        .iter()
        .any(|operator| word.eq_ignore_ascii_case(operator))
}

fn valid_id(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '+' | '_' | ':'))
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    position: usize,
}

impl Parser<'_> {
    fn error(&self, reason: impl Into<String>) -> ParseError {
        ParseError {
            expression: self.source.to_owned(),
            reason: reason.into(),
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn next_is_operator(&self, operator: &str) -> bool {
        matches!(self.peek(), Some(Token::Word(word)) if word.eq_ignore_ascii_case(operator))
    }

    fn or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut branches = vec![self.and_expr()?];
        while self.next_is_operator("OR") || self.peek() == Some(&Token::Slash) {
            self.position += 1;
            branches.push(self.and_expr()?);
        }
        Ok(if branches.len() == 1 {
            branches.remove(0)
        } else {
            Expr::Or(branches)
        })
    }

    fn and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut terms = vec![self.with_expr()?];
        while self.next_is_operator("AND") {
            self.position += 1;
            terms.push(self.with_expr()?);
        }
        Ok(if terms.len() == 1 {
            terms.remove(0)
        } else {
            Expr::And(terms)
        })
    }

    fn with_expr(&mut self) -> Result<Expr, ParseError> {
        let primary = self.primary()?;
        if !self.next_is_operator("WITH") {
            return Ok(primary);
        }
        self.position += 1;
        let Expr::Term(mut term) = primary else {
            return Err(self.error("WITH must follow a single licence, not a group"));
        };
        match self.peek().cloned() {
            Some(Token::Word(word)) if !is_operator(&word) && valid_id(&word) => {
                self.position += 1;
                term.exception = Some(word);
                Ok(Expr::Term(term))
            }
            _ => Err(self.error("WITH needs an exception identifier after it")),
        }
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().cloned() {
            Some(Token::Open) => {
                self.position += 1;
                let inner = self.or_expr()?;
                if self.peek() != Some(&Token::Close) {
                    return Err(self.error("a ( is never closed"));
                }
                self.position += 1;
                Ok(inner)
            }
            Some(Token::Word(word)) if is_operator(&word) => {
                Err(self.error(format!("{word} where a licence was expected")))
            }
            Some(Token::Word(word)) if valid_id(&word) => {
                self.position += 1;
                Ok(Expr::Term(Term {
                    id: word,
                    exception: None,
                }))
            }
            Some(Token::Word(word)) => Err(self.error(format!("{word:?} is not a licence id"))),
            Some(Token::Close) => Err(self.error("a ) has no matching (")),
            Some(Token::Slash) => Err(self.error("a / where a licence was expected")),
            None => Err(self.error("the expression ends where a licence was expected")),
        }
    }
}

/// Parses an SPDX expression (with the old `/` for `OR`).
pub fn parse(text: &str) -> Result<Expr, ParseError> {
    let mut parser = Parser {
        source: text,
        tokens: tokenize(text),
        position: 0,
    };
    if parser.tokens.is_empty() {
        return Err(parser.error("the expression is empty"));
    }
    let expr = parser.or_expr()?;
    match parser.peek() {
        None => Ok(expr),
        Some(Token::Close) => Err(parser.error("a ) has no matching (")),
        Some(other) => Err(parser.error(format!("unexpected {other:?} after the expression"))),
    }
}

/// The lowercase, single-spaced form two spellings of one term share.
pub fn fold_key(key: &str) -> String {
    key.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// The licences Mercury may use a crate under. Matching ignores case; the spelling that was
/// inserted is the one credited.
#[derive(Debug, Clone, Default)]
pub struct TakenSet {
    spelled: BTreeMap<String, String>,
}

impl TakenSet {
    pub fn new<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut set = Self::default();
        for key in keys {
            set.insert(key.as_ref());
        }
        set
    }

    pub fn insert(&mut self, key: &str) {
        let spelled = key.split_whitespace().collect::<Vec<_>>().join(" ");
        self.spelled.entry(fold_key(key)).or_insert(spelled);
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.spelled.contains_key(&fold_key(key))
    }

    pub fn contains(&self, term: &Term) -> bool {
        self.contains_key(&term.key())
    }

    /// The spelling `key` was inserted with.
    pub fn spelling(&self, key: &str) -> Option<&str> {
        self.spelled.get(&fold_key(key)).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.spelled.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spelled.is_empty()
    }
}

impl Expr {
    /// Whether the expression holds when `holds` says which terms do.
    pub fn satisfied(&self, holds: &dyn Fn(&Term) -> bool) -> bool {
        match self {
            Self::Term(term) => holds(term),
            Self::And(branches) => branches.iter().all(|branch| branch.satisfied(holds)),
            Self::Or(branches) => branches.iter().any(|branch| branch.satisfied(holds)),
        }
    }

    /// Every term of the expression, in order.
    pub fn terms(&self) -> Vec<&Term> {
        match self {
            Self::Term(term) => vec![term],
            Self::And(branches) | Self::Or(branches) => {
                branches.iter().flat_map(Self::terms).collect()
            }
        }
    }

    fn credited_set(&self, taken: &TakenSet) -> Option<BTreeSet<String>> {
        match self {
            Self::Term(term) => taken
                .spelling(&term.key())
                .map(|spelled| BTreeSet::from([spelled.to_owned()])),
            Self::And(branches) => {
                let mut all = BTreeSet::new();
                for branch in branches {
                    all.extend(branch.credited_set(taken)?);
                }
                Some(all)
            }
            Self::Or(branches) => {
                let mut any: Option<BTreeSet<String>> = None;
                for branch in branches {
                    if let Some(set) = branch.credited_set(taken) {
                        any.get_or_insert_with(BTreeSet::new).extend(set);
                    }
                }
                any
            }
        }
    }

    /// The licences to credit, sorted: every taken branch of an `OR`, every term of an `AND`.
    /// `None` when the expression cannot be satisfied with the taken licences.
    pub fn credited(&self, taken: &TakenSet) -> Option<Vec<String>> {
        self.credited_set(taken)
            .map(|set| set.into_iter().collect())
    }

    /// Whether Mercury cannot do without licence `id`: it is an `AND` term, or the only taken
    /// branch of an `OR`. (MPL-2.0 then obliges Mercury to point to the source.)
    pub fn requires(&self, taken: &TakenSet, id: &str) -> bool {
        let usable = |expr: &Self| expr.satisfied(&|term| taken.contains(term));
        fn walk(expr: &Expr, id: &str, usable: &dyn Fn(&Expr) -> bool) -> bool {
            match expr {
                Expr::Term(term) => term.exception.is_none() && term.id.eq_ignore_ascii_case(id),
                Expr::And(branches) => branches.iter().any(|branch| walk(branch, id, usable)),
                Expr::Or(branches) => {
                    let usable_branches: Vec<&Expr> =
                        branches.iter().filter(|branch| usable(branch)).collect();
                    !usable_branches.is_empty()
                        && usable_branches
                            .iter()
                            .all(|branch| walk(branch, id, usable))
                }
            }
        }
        usable(self) && walk(self, id, &usable)
    }
}
