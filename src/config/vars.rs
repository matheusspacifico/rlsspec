#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference<'a> {
    Var(&'a str),
    Env(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Raw,
    SqlLiteral,
}

/// Replaces `${name}` and `${env:NAME}` in `input`; `$${` escapes a literal `${`.
pub fn substitute(
    input: &str,
    mode: Mode,
    mut lookup: impl FnMut(Reference) -> Result<String, String>,
) -> Result<String, String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        if let Some(after) = tail.strip_prefix("$${") {
            out.push_str("${");
            rest = after;
        } else if let Some(after) = tail.strip_prefix("${") {
            let end = after
                .find('}')
                .ok_or_else(|| format!("unterminated `${{` in `{input}`"))?;
            let value = lookup(parse_reference(&after[..end])?)?;
            match mode {
                Mode::Raw => out.push_str(&value),
                Mode::SqlLiteral => out.push_str(&quote_literal(&value)),
            }
            rest = &after[end + 1..];
        } else {
            out.push('$');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn parse_reference(body: &str) -> Result<Reference<'_>, String> {
    let reference = match body.strip_prefix("env:") {
        Some(name) => Reference::Env(name),
        None => Reference::Var(body),
    };
    let name = match reference {
        Reference::Var(name) | Reference::Env(name) => name,
    };
    if is_identifier(name) {
        Ok(reference)
    } else {
        Err(format!("invalid variable reference `${{{body}}}`"))
    }
}

pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// Only correct with standard_conforming_strings = on, where backslashes are not escapes;
// the runner must enforce that setting before executing any user predicate.
pub fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(reference: Reference) -> Result<String, String> {
        match reference {
            Reference::Var("tenant") => Ok("t-1".into()),
            Reference::Var("quote") => Ok("o'brien".into()),
            Reference::Env("HOME_DB") => Ok("postgres://localhost/db".into()),
            Reference::Var(name) => Err(format!("undefined variable `{name}`")),
            Reference::Env(name) => Err(format!("environment variable `{name}` is not set")),
        }
    }

    #[test]
    fn raw_substitution() {
        assert_eq!(
            substitute("id ${tenant}!", Mode::Raw, lookup).unwrap(),
            "id t-1!"
        );
        assert_eq!(
            substitute("${env:HOME_DB}", Mode::Raw, lookup).unwrap(),
            "postgres://localhost/db"
        );
    }

    #[test]
    fn sql_literal_substitution_quotes_and_escapes() {
        assert_eq!(
            substitute("tenant_id = ${tenant}", Mode::SqlLiteral, lookup).unwrap(),
            "tenant_id = 't-1'"
        );
        assert_eq!(
            substitute("name = ${quote}", Mode::SqlLiteral, lookup).unwrap(),
            "name = 'o''brien'"
        );
    }

    #[test]
    fn dollars_that_are_not_references_are_kept() {
        assert_eq!(
            substitute("a = $1 and b = $$x$$ and c = $${lit}", Mode::Raw, lookup).unwrap(),
            "a = $1 and b = $$x$$ and c = ${lit}"
        );
    }

    #[test]
    fn errors() {
        assert_eq!(
            substitute("${nope}", Mode::Raw, lookup).unwrap_err(),
            "undefined variable `nope`"
        );
        assert_eq!(
            substitute("${env:NOPE}", Mode::Raw, lookup).unwrap_err(),
            "environment variable `NOPE` is not set"
        );
        assert_eq!(
            substitute("x ${tenant", Mode::Raw, lookup).unwrap_err(),
            "unterminated `${` in `x ${tenant`"
        );
        assert_eq!(
            substitute("${a b}", Mode::Raw, lookup).unwrap_err(),
            "invalid variable reference `${a b}`"
        );
    }
}
