//! Read-only layer 1 — the fail-closed AST check (`_docs/readonly.md`).
//!
//! [`check_read_only`] answers one question about the SQL text it is given: *may this be sent to a
//! connection that must not change anything?* It checks **exactly what it is given**: every
//! statement in the text must pass, and more than one statement is itself a violation
//! ([`ReadOnlyRule::StackedStatements`]) — a caller that runs one statement at a time checks that
//! one statement (`statement_at`) and passes only it.
//!
//! **Fail closed.** Text that does not tokenize or parse is denied; a statement kind that is not
//! explicitly allowed is denied. Allowed are plain queries (`SELECT`, `VALUES`, `WITH … SELECT`),
//! `EXPLAIN` of an allowed query (also `ANALYZE`, since the inner query is then itself allowed),
//! `DESCRIBE`/`SHOW …`, and a short allowlist of SQLite `PRAGMA`s that only report.
//!
//! **Every rule comes from the `sqlparser` AST** (a [`Visitor`] over the statement) with two
//! exceptions that are still `sqlparser`'s own tokenizer, marked `TOKENIZER` below: MySQL
//! executable comments (`/*! … */`, MariaDB `/*M! … */`), which the AST cannot represent because
//! the tokenizer expands them into ordinary tokens; and SQLite `PRAGMA`, whose common form
//! `PRAGMA table_info(t)` 0.63 cannot parse. No regex, no substring matching on SQL text.
//!
//! What the walk denies: data-modifying statements nested in a query (CTEs, subqueries);
//! `SELECT … INTO`; locking clauses; MSSQL lock table hints; optimizer hints; `:=` assignments;
//! and every function call that is not on the per-dialect allowlist in [`super::functions`].
//!
//! Not covered (the parser does not model it, so the text fails to parse and is denied, or it is
//! beyond what layer 1 can see): server-side effects hidden in a *permitted* function's behaviour,
//! `search_path` shadowing of an allowlisted unqualified function, and resource exhaustion — those
//! are layers 2 and 3. `sqlparser`'s recursion guard is off, so pathologically nested input can
//! overflow the stack instead of being denied.

use std::ops::{ControlFlow, Range};

use sqlparser::ast::{
    BinaryOperator, Expr, ObjectName, ObjectNamePart, Query, Select, SetExpr, Statement,
    TableFactor, Visit, Visitor,
};
use sqlparser::dialect::Dialect as SpDialect;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Location, Token, Tokenizer, Whitespace};

use super::{Dialect, Lines, functions, parser_error, split};

/// The verdict of [`check_read_only`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOnlyVerdict {
    Allowed,
    /// Never empty.
    Denied(Vec<Violation>),
}

impl ReadOnlyVerdict {
    pub fn is_allowed(&self) -> bool {
        matches!(self, ReadOnlyVerdict::Allowed)
    }

    /// The violations (empty when allowed).
    pub fn violations(&self) -> &[Violation] {
        match self {
            ReadOnlyVerdict::Allowed => &[],
            ReadOnlyVerdict::Denied(v) => v,
        }
    }
}

/// One reason a text was denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub message: String,
    /// Where in the checked text; the statement's range when the node has no position of its own.
    pub byte_range: Option<Range<usize>>,
    pub rule: ReadOnlyRule,
}

/// Which rule a [`Violation`] broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadOnlyRule {
    /// The text does not tokenize or parse — nothing can be said about it, so it is refused.
    Unparseable,
    /// No statement at all.
    Empty,
    /// More than one statement.
    StackedStatements,
    /// A statement kind that is not on the allowlist (`INSERT`, `SET`, `CALL`, `CREATE`, …).
    StatementKind,
    /// `INSERT`/`UPDATE`/`DELETE`/`MERGE` inside a query: data-modifying CTE or subquery.
    NestedDml,
    /// `SELECT … INTO` (creates a table, writes a file, or sets a variable).
    SelectInto,
    /// `FOR UPDATE`, `FOR SHARE`, `LOCK IN SHARE MODE`, …
    Locking,
    /// A function that is not on the allowlist, or a call that cannot be told apart from a
    /// user-defined one (quoted in MySQL, a foreign schema, a case-sensitive Postgres name).
    Function,
    /// An MSSQL table hint that takes locks or is unknown (`UPDLOCK`, `XLOCK`, `HOLDLOCK`, …).
    TableHint,
    /// An optimizer hint (`/*+ … */`), which can set session variables on MySQL.
    OptimizerHint,
    /// A MySQL/MariaDB executable comment, `/*! … */` or `/*M! … */`.
    ExecutableComment,
    /// A `PRAGMA` that is not an allowlisted reader, or that sets a value.
    Pragma,
    /// A `:=` assignment (MySQL user variables).
    Assignment,
}

/// Check that `sql` can only read. See the module documentation for the rules and what "checks
/// exactly what it is given" means for scripts.
pub fn check_read_only(sql: &str, dialect: Dialect) -> ReadOnlyVerdict {
    let d = dialect.parser_dialect();
    let lines = Lines::new(sql);
    let mut found = Vec::new();

    executable_comments(sql, dialect, &lines, &mut found);

    let split = split(sql, &lines, &*d, dialect);
    let Some(tokens) = &split.tokens else {
        let message = split.error.map_or_else(
            || "the text does not tokenize".to_string(),
            |e| format!("the text does not tokenize: {}", e.message),
        );
        found.push(Violation {
            message,
            byte_range: None,
            rule: ReadOnlyRule::Unparseable,
        });
        return denied(found);
    };
    if split.pieces.is_empty() {
        found.push(Violation {
            message: "no statement to run".into(),
            byte_range: None,
            rule: ReadOnlyRule::Empty,
        });
        return denied(found);
    }

    let mut statements = 0usize;
    for piece in &split.pieces {
        let chunk = tokens[piece.tokens.clone()].to_vec();
        let significant: Vec<&Token> = chunk
            .iter()
            .map(|t| &t.token)
            .filter(|t| !matches!(t, Token::Whitespace(_)))
            .collect();
        if matches!(dialect, Dialect::Sqlite | Dialect::Generic)
            && matches!(significant.first(), Some(Token::Word(w))
                if w.quote_style.is_none() && w.value.eq_ignore_ascii_case("pragma"))
        {
            statements += 1;
            Checker::new(&lines, dialect, piece.range.clone(), &mut found)
                .pragma(&significant[1..]);
            continue;
        }
        match Parser::new(&*d)
            .with_tokens_with_locations(chunk)
            .parse_statements()
        {
            Ok(stmts) => {
                statements += stmts.len();
                for stmt in &stmts {
                    let mut c = Checker::new(&lines, dialect, piece.range.clone(), &mut found);
                    c.statement(stmt);
                }
            }
            Err(e) => {
                statements += 1;
                found.push(Violation {
                    message: format!(
                        "does not parse, so it cannot be shown read-only: {}",
                        parser_error(&e, &lines, piece.range.end).message
                    ),
                    byte_range: Some(piece.range.clone()),
                    rule: ReadOnlyRule::Unparseable,
                });
            }
        }
    }
    if statements > 1 {
        found.push(Violation {
            message: format!("{statements} statements; run one statement at a time"),
            byte_range: Some(split.pieces[1].range.clone()),
            rule: ReadOnlyRule::StackedStatements,
        });
    }
    denied(found)
}

fn denied(mut found: Vec<Violation>) -> ReadOnlyVerdict {
    if found.is_empty() {
        return ReadOnlyVerdict::Allowed;
    }
    // The walk can reach one node twice (a nested statement is both a `SetExpr` and a statement).
    let mut seen = Vec::<(ReadOnlyRule, Option<Range<usize>>, String)>::new();
    found.retain(|v| {
        let key = (v.rule, v.byte_range.clone(), v.message.clone());
        !seen.contains(&key) && {
            seen.push(key);
            true
        }
    });
    ReadOnlyVerdict::Denied(found)
}

// ---- TOKENIZER: executable comments ---------------------------------------------------------

/// The dialect's own tokenizer *expands* `/*! … */` into live tokens, so the comment itself never
/// shows up in the token stream (and the AST then simply contains what MySQL would execute). To
/// see the comment we tokenize once more with a copy of the dialect that does not expand it.
// TOKENIZER: the one place the AST cannot answer, and it is still `sqlparser`.
fn executable_comments(sql: &str, dialect: Dialect, lines: &Lines, out: &mut Vec<Violation>) {
    let plain = KeepComments(dialect.parser_dialect());
    let Ok(tokens) = Tokenizer::new(&plain, sql).tokenize_with_location() else {
        return; // the main tokenization reports the failure
    };
    for t in tokens {
        if let Token::Whitespace(Whitespace::MultiLineComment(c)) = &t.token
            && (c.starts_with('!') || c.starts_with("M!"))
        {
            out.push(Violation {
                message: "executable comment (/*! … */): the server runs its content".into(),
                byte_range: Some(lines.offset(t.span.start)..lines.token_end(&t)),
                rule: ReadOnlyRule::ExecutableComment,
            });
        }
    }
}

/// A dialect identical to the wrapped one for tokenizing, except that `/*! … */` stays a comment.
#[derive(Debug)]
struct KeepComments(Box<dyn SpDialect>);

impl SpDialect for KeepComments {
    fn dialect(&self) -> std::any::TypeId {
        self.0.dialect()
    }
    fn is_identifier_start(&self, ch: char) -> bool {
        self.0.is_identifier_start(ch)
    }
    fn is_identifier_part(&self, ch: char) -> bool {
        self.0.is_identifier_part(ch)
    }
    fn is_delimited_identifier_start(&self, ch: char) -> bool {
        self.0.is_delimited_identifier_start(ch)
    }
    fn is_custom_operator_part(&self, ch: char) -> bool {
        self.0.is_custom_operator_part(ch)
    }
    fn ignores_wildcard_escapes(&self) -> bool {
        self.0.ignores_wildcard_escapes()
    }
    fn requires_single_line_comment_whitespace(&self) -> bool {
        self.0.requires_single_line_comment_whitespace()
    }
    fn supports_dollar_as_money_prefix(&self) -> bool {
        self.0.supports_dollar_as_money_prefix()
    }
    fn supports_dollar_placeholder(&self) -> bool {
        self.0.supports_dollar_placeholder()
    }
    fn supports_geometric_types(&self) -> bool {
        self.0.supports_geometric_types()
    }
    fn supports_multiline_comment_hints(&self) -> bool {
        false
    }
    fn supports_nested_comments(&self) -> bool {
        self.0.supports_nested_comments()
    }
    fn supports_numeric_literal_underscores(&self) -> bool {
        self.0.supports_numeric_literal_underscores()
    }
    fn supports_numeric_prefix(&self) -> bool {
        self.0.supports_numeric_prefix()
    }
    fn supports_pipe_operator(&self) -> bool {
        self.0.supports_pipe_operator()
    }
    fn supports_quote_delimited_string(&self) -> bool {
        self.0.supports_quote_delimited_string()
    }
    fn supports_string_escape_constant(&self) -> bool {
        self.0.supports_string_escape_constant()
    }
    fn supports_string_literal_backslash_escape(&self) -> bool {
        self.0.supports_string_literal_backslash_escape()
    }
    fn supports_triple_quoted_string(&self) -> bool {
        self.0.supports_triple_quoted_string()
    }
    fn supports_unicode_string_literal(&self) -> bool {
        self.0.supports_unicode_string_literal()
    }
}

// ---- the statement check and the AST walk ---------------------------------------------------

/// Checks one statement; doubles as the [`Visitor`] for the walk below it.
struct Checker<'a> {
    lines: &'a Lines<'a>,
    dialect: Dialect,
    /// The statement's range, used for violations whose node has no position of its own.
    stmt: Range<usize>,
    out: &'a mut Vec<Violation>,
    /// Statements entered by the visitor; the walk's root is depth 1, anything deeper is nested.
    depth: usize,
}

impl<'a> Checker<'a> {
    fn new(
        lines: &'a Lines<'a>,
        dialect: Dialect,
        stmt: Range<usize>,
        out: &'a mut Vec<Violation>,
    ) -> Self {
        Checker {
            lines,
            dialect,
            stmt,
            out,
            depth: 0,
        }
    }

    fn add(&mut self, rule: ReadOnlyRule, message: String, range: Option<Range<usize>>) {
        self.out.push(Violation {
            message,
            byte_range: Some(range.unwrap_or_else(|| self.stmt.clone())),
            rule,
        });
    }

    /// The statement-kind allowlist.
    fn statement(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Query(_)
            | Statement::ExplainTable { .. }
            | Statement::ShowVariable { .. }
            | Statement::ShowVariables { .. }
            | Statement::ShowStatus { .. }
            | Statement::ShowCreate { .. }
            | Statement::ShowColumns { .. }
            | Statement::ShowDatabases { .. }
            | Statement::ShowSchemas { .. }
            | Statement::ShowCatalogs { .. }
            | Statement::ShowTables { .. }
            | Statement::ShowViews { .. }
            | Statement::ShowFunctions { .. }
            | Statement::ShowCollation { .. }
            | Statement::ShowCharset(_)
            | Statement::ShowObjects(_)
            | Statement::ShowProcessList { .. } => self.walk(stmt),
            // `EXPLAIN ANALYZE` executes its statement, so the statement decides — and only a
            // query is accepted there, never a write that is "only" being explained.
            Statement::Explain { statement, .. } => match &**statement {
                Statement::Query(_) => self.statement(statement),
                other => self.add(
                    ReadOnlyRule::StatementKind,
                    format!("EXPLAIN of {}: only a query may be explained", label(other)),
                    None,
                ),
            },
            other => self.add(
                ReadOnlyRule::StatementKind,
                format!("{} is not a read-only statement", label(other)),
                None,
            ),
        }
    }

    fn walk(&mut self, stmt: &Statement) {
        let _ = stmt.visit(self);
    }

    /// `PRAGMA [schema.]name` or `PRAGMA [schema.]name(arg)` with `name` on the read allowlist.
    /// `rest` is the statement's significant tokens after `PRAGMA`.
    // TOKENIZER: `sqlparser` 0.63 cannot represent `PRAGMA table_info(t)` (it wants a literal
    // inside the parentheses), so the common read form is judged from its tokens.
    fn pragma(&mut self, rest: &[&Token]) {
        use Token::{LParen, Number, Period, RParen, SingleQuotedString, Word};
        let (name, arg) = match rest {
            [Word(n)] | [Word(_), Period, Word(n)] => (n, false),
            [
                Word(n),
                LParen,
                Word(_) | SingleQuotedString(_) | Number(..),
                RParen,
            ]
            | [
                Word(_),
                Period,
                Word(n),
                LParen,
                Word(_) | SingleQuotedString(_) | Number(..),
                RParen,
            ] => (n, true),
            _ => return self.deny_pragma(rest),
        };
        let word = name.value.to_lowercase();
        let listed = |table: &str| table.split_whitespace().any(|w| w == word);
        let reads =
            listed(functions::PRAGMAS_WITH_ARG) || (!arg && listed(functions::PRAGMAS_NO_ARG));
        if !reads {
            self.deny_pragma(rest);
        }
    }

    fn deny_pragma(&mut self, rest: &[&Token]) {
        let text: Vec<String> = rest.iter().map(|t| t.to_string()).collect();
        self.add(
            ReadOnlyRule::Pragma,
            format!(
                "PRAGMA {} is not an allowlisted read pragma",
                text.join(" ")
            ),
            None,
        );
    }

    /// A function call by name: allowed only if unqualified (or `pg_catalog.` on Postgres) and on
    /// the dialect's allowlist.
    fn function(&mut self, name: &ObjectName) {
        if !self.function_allowed(name) {
            self.add(
                ReadOnlyRule::Function,
                format!("function {name} is not on the read-only allowlist"),
                self.name_range(name),
            );
        }
    }

    fn function_allowed(&self, name: &ObjectName) -> bool {
        let idents: Option<Vec<_>> = name.0.iter().map(ObjectNamePart::as_ident).collect();
        let Some(idents) = idents else {
            return false; // `IDENTIFIER(…)` style computed names
        };
        let postgres = matches!(self.dialect, Dialect::Postgres | Dialect::Generic);
        let last = match idents.as_slice() {
            [last] => *last,
            [q, last] if postgres && q.value.eq_ignore_ascii_case("pg_catalog") => *last,
            _ => return false,
        };
        if last.quote_style.is_some() {
            match self.dialect {
                // MySQL: a quoted name before `(` is a stored function, never a built-in.
                Dialect::MySql => return false,
                // Postgres: quoted names are case-sensitive; `"COUNT"` is not `count`.
                Dialect::Postgres if last.value != last.value.to_lowercase() => return false,
                _ => {}
            }
        }
        functions::is_allowed(self.dialect, &last.value.to_lowercase())
    }

    fn name_range(&self, name: &ObjectName) -> Option<Range<usize>> {
        let spans = name
            .0
            .iter()
            .filter_map(ObjectNamePart::as_ident)
            .map(|i| i.span);
        let (mut first, mut last) = (None::<Location>, None::<Location>);
        for s in spans.filter(|s| s.start.line > 0) {
            first.get_or_insert(s.start);
            last = Some(s.end);
        }
        Some(self.lines.offset(first?)..self.lines.offset(last?))
    }
}

impl Visitor for Checker<'_> {
    type Break = ();

    fn pre_visit_statement(&mut self, stmt: &Statement) -> ControlFlow<()> {
        self.depth += 1;
        if self.depth > 1 {
            self.add(
                ReadOnlyRule::NestedDml,
                format!("{} nested inside the statement", label(stmt)),
                None,
            );
        }
        ControlFlow::Continue(())
    }

    fn post_visit_statement(&mut self, _: &Statement) -> ControlFlow<()> {
        self.depth -= 1;
        ControlFlow::Continue(())
    }

    fn pre_visit_query(&mut self, q: &Query) -> ControlFlow<()> {
        if !q.locks.is_empty() {
            self.add(
                ReadOnlyRule::Locking,
                "locking clause (FOR UPDATE / FOR SHARE / LOCK IN SHARE MODE)".into(),
                None,
            );
        }
        if q.settings.is_some() || q.format_clause.is_some() {
            self.add(
                ReadOnlyRule::StatementKind,
                "SETTINGS / FORMAT clauses are not read-only".into(),
                None,
            );
        }
        if let SetExpr::Insert(s) | SetExpr::Update(s) | SetExpr::Delete(s) | SetExpr::Merge(s) =
            &*q.body
        {
            self.add(
                ReadOnlyRule::NestedDml,
                format!("data-modifying {} inside a query", label(s)),
                None,
            );
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_select(&mut self, s: &Select) -> ControlFlow<()> {
        if s.into.is_some() {
            self.add(
                ReadOnlyRule::SelectInto,
                "SELECT … INTO creates a table, writes a file or sets a variable".into(),
                None,
            );
        }
        if !s.optimizer_hints.is_empty() {
            self.add(
                ReadOnlyRule::OptimizerHint,
                "optimizer hints are not allowed (they can set session variables)".into(),
                None,
            );
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
        match e {
            Expr::Function(f) => self.function(&f.name),
            Expr::BinaryOp {
                op: BinaryOperator::Assignment,
                ..
            } => self.add(
                ReadOnlyRule::Assignment,
                "`:=` assigns a user variable".into(),
                None,
            ),
            _ => {}
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, t: &TableFactor) -> ControlFlow<()> {
        match t {
            // A table with arguments is a table-valued function: `FROM generate_series(…)`.
            TableFactor::Table {
                name,
                args: Some(_),
                ..
            }
            | TableFactor::Function { name, .. } => self.function(name),
            TableFactor::Table { with_hints, .. } => {
                for hint in with_hints {
                    let ok = matches!(hint, Expr::Identifier(i) if functions::is_allowed_table_hint(&i.value));
                    if !ok {
                        self.add(
                            ReadOnlyRule::TableHint,
                            format!("table hint {hint} may take locks"),
                            None,
                        );
                    }
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

/// A statement's leading keywords, for messages only (the decision never depends on it).
fn label(stmt: &Statement) -> String {
    stmt.to_string()
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}
