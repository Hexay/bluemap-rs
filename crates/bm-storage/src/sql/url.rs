//! BlueMap's JDBC `connection-url` + `connection-properties` → a sqlx URL.

use super::Dialect;
use crate::error::{Error, Result};

impl Dialect {
    /// Dialect and sqlx URL for a sqlx-style or BlueMap JDBC-style (`jdbc:mysql://…`) connection URL.
    pub fn from_url(url: &str) -> Result<(Self, String)> {
        let bare = url.strip_prefix("jdbc:").unwrap_or(url);
        let (scheme, rest) = bare.split_once(':').ok_or_else(|| Error::UnsupportedUrl(url.to_owned()))?;
        let dialect = match scheme {
            "mysql" | "mariadb" => Self::MySql,
            "postgres" | "postgresql" => Self::Postgres,
            "sqlite" => Self::Sqlite,
            _ => return Err(Error::UnsupportedUrl(url.to_owned())),
        };
        let scheme = if dialect == Self::MySql { "mysql" } else { scheme };
        Ok((dialect, format!("{scheme}:{rest}")))
    }
}

/// Dialect and sqlx URL. `user`/`password` come from `properties` or the JDBC query string and become
/// percent-encoded userinfo (sqlx' MySQL driver silently ignores them as query parameters); JDBC spellings of SSL
/// and schema options are translated, everything else is passed through (sqlx ignores what it doesn't know).
pub(crate) fn connect_url(url: &str, properties: &[(String, String)]) -> Result<(Dialect, String)> {
    let (dialect, url) = Dialect::from_url(url)?;
    let Some(scheme_end) = url.find("://").map(|i| i + 3) else { return Ok((dialect, url)) };
    let (head, query) = match url.split_once('?') {
        Some((head, query)) => (head, query),
        None => (url.as_str(), ""),
    };
    let mut params: Vec<String> = Vec::new();
    let (mut user, mut password) = (None, None);
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match (dialect, k) {
            (_, "user") => user = Some(decode(v)),
            (_, "password") => password = Some(decode(v)),
            (Dialect::MySql, "sslMode") => params.push(format!("ssl-mode={v}")),
            (Dialect::MySql, "useSSL") if v.eq_ignore_ascii_case("false") => params.push("ssl-mode=DISABLED".into()),
            (Dialect::Postgres, "currentSchema") => params.push(format!("options[search_path]={v}")),
            _ => params.push(pair.into()),
        }
    }
    for (k, v) in properties {
        match k.as_str() {
            "user" => user = Some(v.clone()),
            "password" => password = Some(v.clone()),
            _ => {}
        }
    }
    let mut out = head[..scheme_end].to_owned();
    let has_userinfo = head[scheme_end..].split('/').next().is_some_and(|authority| authority.contains('@'));
    if let (Some(user), false) = (&user, has_userinfo) {
        out.push_str(&encode(user));
        if let Some(pw) = &password {
            out.push(':');
            out.push_str(&encode(pw));
        }
        out.push('@');
    }
    out.push_str(&head[scheme_end..]);
    if !params.is_empty() {
        out.push('?');
        out.push_str(&params.join("&"));
    }
    Ok((dialect, out))
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                i += 1;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn dialects() {
        assert_eq!(Dialect::from_url("jdbc:mysql://h:3306/db").unwrap(), (Dialect::MySql, "mysql://h:3306/db".into()));
        assert_eq!(Dialect::from_url("jdbc:mariadb://h/db").unwrap(), (Dialect::MySql, "mysql://h/db".into()));
        assert_eq!(Dialect::from_url("jdbc:postgresql://h/db").unwrap().0, Dialect::Postgres);
        assert_eq!(Dialect::from_url("sqlite:bluemap.db").unwrap(), (Dialect::Sqlite, "sqlite:bluemap.db".into()));
        assert!(Dialect::from_url("jdbc:oracle:thin:@h").is_err());
    }

    #[test]
    fn credentials_from_properties_are_encoded() {
        let p = props(&[("user", "blue map"), ("password", "p@ss:w/rd%")]);
        let (_, url) = connect_url("jdbc:mysql://localhost:3306/bluemap?permitMysqlScheme", &p).unwrap();
        assert_eq!(url, "mysql://blue%20map:p%40ss%3Aw%2Frd%25@localhost:3306/bluemap?permitMysqlScheme");
        let (_, url) = connect_url("mysql://a:b@h/db", &p).unwrap();
        assert_eq!(url, "mysql://a:b@h/db", "explicit userinfo wins");
    }

    #[test]
    fn jdbc_query_parameters() {
        let (_, url) = connect_url("jdbc:mariadb://h/db?user=bm&password=a%26b&useSSL=false&useUnicode=true", &[]).unwrap();
        assert_eq!(url, "mysql://bm:a%26b@h/db?ssl-mode=DISABLED&useUnicode=true");
        let (_, url) = connect_url("jdbc:postgresql://h:5432/db?currentSchema=maps&sslmode=require", &props(&[("user", "u")])).unwrap();
        assert_eq!(url, "postgresql://u@h:5432/db?options[search_path]=maps&sslmode=require");
        assert_eq!(connect_url("sqlite:bm.db", &props(&[("user", "u")])).unwrap().1, "sqlite:bm.db");
    }
}
