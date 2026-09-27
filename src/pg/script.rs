use std::ops::Range;

use crate::config::{Diagnostic, Span};

const FORBIDDEN: [&str; 8] = [
    "BEGIN",
    "START",
    "COMMIT",
    "END",
    "ROLLBACK",
    "ABORT",
    "SAVEPOINT",
    "RELEASE",
];

/// Finds top-level transaction control statements in a multi-statement SQL script.
/// Strings, quoted identifiers, comments and dollar-quoted bodies are skipped.
pub fn transaction_control(sql: &str) -> Vec<Diagnostic> {
    scan(sql).diagnostics
}

/// The byte ranges of the top-level statements of a script, each from its first token to its `;`.
pub fn statements(sql: &str) -> Vec<Range<usize>> {
    scan(sql).statements
}

/// The 1-based line and column (in characters) of byte offset `at`.
pub fn span_at(sql: &str, at: usize) -> Span {
    let before = &sql[..at];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    Span {
        line: before.matches('\n').count() + 1,
        column: before[line_start..].chars().count() + 1,
    }
}

fn scan(sql: &str) -> Scanner<'_> {
    let mut scanner = Scanner {
        sql,
        bytes: sql.as_bytes(),
        pos: 0,
        diagnostics: Vec::new(),
        statements: Vec::new(),
        statement: Statement::default(),
    };
    scanner.run();
    if let Some(start) = scanner.statement.start {
        scanner.statements.push(start..sql.len());
    }
    scanner
}

#[derive(Default)]
struct Statement {
    start: Option<usize>,
    words: Vec<(String, usize)>,
    started: bool,
    routine: bool,
    depth: usize,
}

struct Scanner<'a> {
    sql: &'a str,
    bytes: &'a [u8],
    pos: usize,
    diagnostics: Vec<Diagnostic>,
    statements: Vec<Range<usize>>,
    statement: Statement,
}

impl Scanner<'_> {
    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn mark(&mut self, at: usize) {
        self.statement.start.get_or_insert(at);
    }

    fn run(&mut self) {
        while let Some(c) = self.peek(0) {
            match c {
                b'-' if self.peek(1) == Some(b'-') => self.line_comment(),
                b'/' if self.peek(1) == Some(b'*') => self.block_comment(),
                b'\'' => self.string(false),
                b'"' => self.quoted_identifier(),
                b'$' if self.dollar_tag().is_some() => self.dollar_body(),
                b';' => {
                    self.pos += 1;
                    if self.statement.depth == 0 {
                        if let Some(start) = self.statement.start {
                            self.statements.push(start..self.pos);
                        }
                        self.statement = Statement::default();
                    }
                }
                c if c.is_ascii_whitespace() => self.pos += 1,
                c if is_word_start(c) => self.word(),
                _ => {
                    self.mark(self.pos);
                    self.statement.started = true;
                    self.pos += 1;
                }
            }
        }
    }

    fn line_comment(&mut self) {
        while let Some(c) = self.peek(0) {
            if c == b'\n' {
                break;
            }
            self.pos += 1;
        }
    }

    fn block_comment(&mut self) {
        let mut depth = 0usize;
        while self.pos < self.bytes.len() {
            if self.peek(0) == Some(b'/') && self.peek(1) == Some(b'*') {
                depth += 1;
                self.pos += 2;
            } else if self.peek(0) == Some(b'*') && self.peek(1) == Some(b'/') {
                depth -= 1;
                self.pos += 2;
                if depth == 0 {
                    return;
                }
            } else {
                self.pos += 1;
            }
        }
    }

    fn string(&mut self, backslash_escapes: bool) {
        self.mark(self.pos);
        self.statement.started = true;
        self.pos += 1;
        while let Some(c) = self.peek(0) {
            self.pos += 1;
            match c {
                b'\\' if backslash_escapes => self.pos += 1,
                b'\'' if self.peek(0) == Some(b'\'') => self.pos += 1,
                b'\'' => return,
                _ => {}
            }
        }
    }

    fn quoted_identifier(&mut self) {
        self.mark(self.pos);
        self.statement.started = true;
        self.pos += 1;
        while let Some(c) = self.peek(0) {
            self.pos += 1;
            if c == b'"' {
                if self.peek(0) == Some(b'"') {
                    self.pos += 1;
                } else {
                    return;
                }
            }
        }
    }

    fn dollar_tag(&self) -> Option<usize> {
        let rest = &self.bytes[self.pos + 1..];
        let len = rest.iter().take_while(|&&c| is_word_start(c)).count();
        let starts_with_digit = rest.first().is_some_and(u8::is_ascii_digit);
        (len == 0 || !starts_with_digit)
            .then_some(len + 2)
            .filter(|_| rest.get(len) == Some(&b'$'))
    }

    fn dollar_body(&mut self) {
        self.mark(self.pos);
        self.statement.started = true;
        let Some(len) = self.dollar_tag() else {
            return;
        };
        let delimiter = &self.sql[self.pos..self.pos + len];
        let body = self.pos + len;
        self.pos = match self.sql[body..].find(delimiter) {
            Some(end) => body + end + len,
            None => self.bytes.len(),
        };
    }

    fn word(&mut self) {
        let start = self.pos;
        self.mark(start);
        while self.peek(0).is_some_and(is_word_char) {
            self.pos += 1;
        }
        let word = self.sql[start..self.pos].to_ascii_uppercase();
        if word == "E" && self.peek(0) == Some(b'\'') {
            self.string(true);
            return;
        }
        self.keyword(word, start);
    }

    fn keyword(&mut self, word: String, at: usize) {
        let statement = &mut self.statement;
        if statement.routine {
            match word.as_str() {
                "BEGIN" => statement.depth += 1,
                "CASE" if statement.depth > 0 => statement.depth += 1,
                "END" if statement.depth > 0 => statement.depth -= 1,
                _ => {}
            }
        }
        if statement.started && statement.words.len() >= 4 {
            return;
        }
        let first = !statement.started;
        statement.started = true;
        statement.words.push((word, at));
        let words: Vec<&str> = statement.words.iter().map(|(w, _)| w.as_str()).collect();
        statement.routine = matches!(
            words.as_slice(),
            ["CREATE", "FUNCTION" | "PROCEDURE", ..]
                | ["CREATE", "OR", "REPLACE", "FUNCTION" | "PROCEDURE"]
        );
        let forbidden = match words.as_slice() {
            [first_word] if first && FORBIDDEN.contains(first_word) => Some(first_word.to_string()),
            ["PREPARE", "TRANSACTION"] => Some("PREPARE TRANSACTION".to_owned()),
            _ => None,
        };
        if let Some(keyword) = forbidden {
            let at = statement.words[0].1;
            self.report(&keyword, at);
        }
    }

    fn report(&mut self, keyword: &str, at: usize) {
        self.diagnostics.push(Diagnostic {
            span: span_at(self.sql, at),
            message: format!(
                "`{keyword}` is not allowed in setup files: rlsspec runs everything in one transaction that is always rolled back"
            ),
        });
    }
}

fn is_word_start(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

fn is_word_char(c: u8) -> bool {
    is_word_start(c) || c == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(sql: &str) -> Vec<(usize, usize, String)> {
        transaction_control(sql)
            .into_iter()
            .map(|d| {
                let keyword = d.message.split('`').nth(1).unwrap_or_default().to_owned();
                (d.span.line, d.span.column, keyword)
            })
            .collect()
    }

    fn allowed(sql: &str) {
        assert_eq!(found(sql), [], "{sql}");
    }

    #[test]
    fn every_transaction_control_statement_is_rejected() {
        for (sql, keyword) in [
            ("begin;", "BEGIN"),
            ("BEGIN ISOLATION LEVEL SERIALIZABLE;", "BEGIN"),
            ("start transaction;", "START"),
            ("commit;", "COMMIT"),
            ("COMMIT AND CHAIN;", "COMMIT"),
            ("end;", "END"),
            ("rollback;", "ROLLBACK"),
            ("ROLLBACK TO SAVEPOINT a;", "ROLLBACK"),
            ("abort;", "ABORT"),
            ("savepoint a;", "SAVEPOINT"),
            ("release savepoint a;", "RELEASE"),
            ("prepare transaction 'x';", "PREPARE TRANSACTION"),
            ("commit prepared 'x';", "COMMIT"),
            ("rollback prepared 'x';", "ROLLBACK"),
            ("Commit", "COMMIT"),
        ] {
            assert_eq!(found(sql), [(1, 1, keyword.to_owned())], "{sql}");
        }
    }

    #[test]
    fn location_points_at_the_statement() {
        assert_eq!(
            found("insert into t values (1);\n\n  -- done\n  commit;\nselect 1; end"),
            [(4, 3, "COMMIT".into()), (5, 11, "END".into())]
        );
        assert_eq!(
            found("select 'é'; /* x */ rollback;"),
            [(1, 21, "ROLLBACK".into())]
        );
    }

    #[test]
    fn keywords_inside_literals_and_comments_are_allowed() {
        allowed("select 'commit; begin';");
        allowed("select 'it''s; commit';");
        allowed("select E'\\'; commit';");
        allowed("select e'\\\\'; select 1;");
        allowed("select U&'d\\0061t; commit';");
        allowed("select \"commit;\" from t;");
        allowed("select \"a\"\"; commit\" from t;");
        allowed("-- commit;\nselect 1;");
        allowed("/* commit; */ select 1;");
        allowed("/* outer /* commit; */ still comment; commit; */ select 1;");
        allowed("do $$ begin commit; end $$;");
        allowed(
            "create function f() returns void language plpgsql as $fn$\nbegin\n  commit;\nend\n$fn$;",
        );
        allowed("select $a$ $b$ commit; $b$ $a$;");
    }

    #[test]
    fn keywords_not_at_statement_start_are_allowed() {
        allowed("select 1 as begin;");
        allowed("create table t (commit int, \"end\" int);");
        allowed("select case when true then 1 end;");
        allowed("prepare p as select 1;");
        allowed("select $1, a$b from t;");
    }

    #[test]
    fn sql_standard_function_bodies_are_one_statement() {
        allowed(
            "create function f(x int) returns int language sql\nbegin atomic\n  select case when x > 0 then x end;\n  select x;\nend;\nselect f(1);",
        );
        allowed(
            "CREATE OR REPLACE PROCEDURE p() LANGUAGE sql BEGIN ATOMIC insert into t values (1); END;",
        );
        assert_eq!(
            found("create procedure p() language sql begin atomic select 1; end;\ncommit;"),
            [(2, 1, "COMMIT".into())]
        );
    }

    fn split(sql: &str) -> Vec<&str> {
        statements(sql).into_iter().map(|r| &sql[r]).collect()
    }

    #[test]
    fn statements_are_split_at_top_level_semicolons() {
        assert_eq!(
            split("-- seed\ninsert into t values (';');\n\n/* x */ select 1;select 2"),
            ["insert into t values (';');", "select 1;", "select 2"]
        );
        assert_eq!(
            split(
                "create function f() returns int language sql\nbegin atomic select 1; end;\ndo $$ begin null; end $$;"
            ),
            [
                "create function f() returns int language sql\nbegin atomic select 1; end;",
                "do $$ begin null; end $$;"
            ]
        );
        assert_eq!(split("E'a;b'; \"x;\" ;"), ["E'a;b';", "\"x;\" ;"]);
        assert_eq!(split(";; -- nothing\n"), Vec::<&str>::new());
    }

    #[test]
    fn span_at_counts_characters() {
        let sql = "select 'é';\n  x";
        assert_eq!(
            span_at(sql, sql.find('x').unwrap_or(0)),
            Span { line: 2, column: 3 }
        );
        assert_eq!(span_at("é,", 2), Span { line: 1, column: 2 });
    }

    #[test]
    fn unterminated_constructs_do_not_panic() {
        allowed("select 'abc");
        allowed("select \"abc");
        allowed("/* abc");
        allowed("select $$abc");
        allowed("select E'\\");
    }
}
