use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    Rls001,
    Rls002,
    Rls003,
    Rls004,
    Rls005,
    Rls006,
    Rls007,
    Rls008,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warn,
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warn => "warn",
            Severity::Info => "info",
        }
    }
}

/// What a `lint.ignore` entry can match a finding on, besides its rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Table,
    Identity,
    Function,
    View,
}

impl Key {
    pub fn as_str(self) -> &'static str {
        match self {
            Key::Table => "table",
            Key::Identity => "identity",
            Key::Function => "function",
            Key::View => "view",
        }
    }
}

impl Rule {
    pub const ALL: [Rule; 8] = [
        Rule::Rls001,
        Rule::Rls002,
        Rule::Rls003,
        Rule::Rls004,
        Rule::Rls005,
        Rule::Rls006,
        Rule::Rls007,
        Rule::Rls008,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Rule::Rls001 => "RLS001",
            Rule::Rls002 => "RLS002",
            Rule::Rls003 => "RLS003",
            Rule::Rls004 => "RLS004",
            Rule::Rls005 => "RLS005",
            Rule::Rls006 => "RLS006",
            Rule::Rls007 => "RLS007",
            Rule::Rls008 => "RLS008",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|rule| rule.id() == id)
    }

    pub fn severity(self) -> Severity {
        match self {
            Rule::Rls001 | Rule::Rls002 | Rule::Rls003 => Severity::Error,
            Rule::Rls004 | Rule::Rls005 | Rule::Rls007 | Rule::Rls008 => Severity::Warn,
            Rule::Rls006 => Severity::Info,
        }
    }

    pub fn keys(self) -> &'static [Key] {
        match self {
            Rule::Rls001 | Rule::Rls006 => &[Key::Table],
            Rule::Rls002 | Rule::Rls004 | Rule::Rls008 => &[Key::Table, Key::Identity],
            Rule::Rls003 => &[Key::Identity],
            Rule::Rls005 => &[Key::Function, Key::Identity],
            Rule::Rls007 => &[Key::View, Key::Identity],
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.id())
    }
}
