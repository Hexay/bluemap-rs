use crate::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Ws(String),
    Newline,
    Comma,
    Colon,
    Equals,
    PlusEquals,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Quoted(String),
    Unquoted(String),
    Number { text: String, value: Value },
    Subst { path: Vec<String>, optional: bool },
    Eof,
}

impl Tok {
    pub(crate) fn describe(&self) -> String {
        match self {
            Tok::Ws(_) => "whitespace".into(),
            Tok::Newline => "a newline".into(),
            Tok::Comma => "','".into(),
            Tok::Colon => "':'".into(),
            Tok::Equals => "'='".into(),
            Tok::PlusEquals => "'+='".into(),
            Tok::LBrace => "'{'".into(),
            Tok::RBrace => "'}'".into(),
            Tok::LBracket => "'['".into(),
            Tok::RBracket => "']'".into(),
            Tok::Quoted(s) => format!("\"{s}\""),
            Tok::Unquoted(s) | Tok::Number { text: s, .. } => format!("'{s}'"),
            Tok::Subst { path, .. } => format!("${{{}}}", path.join(".")),
            Tok::Eof => "end of file".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
}
