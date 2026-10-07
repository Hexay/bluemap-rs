//! One test per HOCON feature, plus error positions/messages (BlueMap issues #654/#735: unhelpful errors).

use bm_config::{ParseError, Value, hocon};

fn parse(src: &str) -> Value {
    Value::Object(hocon::parse_str(src, "t.conf").unwrap_or_else(|e| panic!("{e}")))
}

fn err(src: &str) -> ParseError {
    hocon::parse_str(src, "t.conf").unwrap_err()
}

fn at<'a>(v: &'a Value, path: &str) -> &'a Value {
    v.at(&path.split('.').collect::<Vec<_>>()).unwrap_or_else(|| panic!("no {path}"))
}

fn s(x: &str) -> Value {
    Value::String(x.into())
}

#[test]
fn comments_and_separators() {
    let v = parse("# hash\n// slashes\na: 1 # trailing\nb = 2 // trailing\nc { d: 3 }\nurl: \"http://x//y\"");
    assert_eq!((at(&v, "a"), at(&v, "b"), at(&v, "c.d")), (&Value::Int(1), &Value::Int(2), &Value::Int(3)));
    assert_eq!(at(&v, "url"), &s("http://x//y"));
}

#[test]
fn keys() {
    let v = parse("a.b.c: 1\n\"x.y\": 2\nk.\"p.q\": 3\nfoo bar: 4\n10.5: 5");
    assert_eq!(at(&v, "a.b.c"), &Value::Int(1));
    assert_eq!(v.get("x.y"), Some(&Value::Int(2)));
    assert_eq!(v.get("k").unwrap().get("p.q"), Some(&Value::Int(3)));
    assert_eq!(v.get("foo bar"), Some(&Value::Int(4)));
    assert_eq!(at(&v, "10.5"), &Value::Int(5));
}

#[test]
fn strings() {
    let v = parse(
        "a: \"q\\\"\\u0041\\n\"\nb: unquoted text  here\nc: \"\"\"multi\nline \"x\" \"\"\"\nd: \"\"\"a\"\"\"\"\ne: foo\"bar\"",
    );
    assert_eq!(at(&v, "a"), &s("q\"A\n"));
    assert_eq!(at(&v, "b"), &s("unquoted text  here"));
    assert_eq!(at(&v, "c"), &s("multi\nline \"x\" "));
    assert_eq!(at(&v, "d"), &s("a\""));
    assert_eq!(at(&v, "e"), &s("foobar"));
}

#[test]
fn scalars() {
    let v =
        parse("i: -5\nl: 9999999999\nf: 1.5e2\ng: 1.25\nt: true\nn: null\nver: 1.0.0\nip: 192.168.0.1\nmix: 10abc\nlead: 007");
    assert_eq!(at(&v, "i"), &Value::Int(-5));
    assert_eq!(at(&v, "l"), &Value::Int(9_999_999_999));
    // typesafe stores whole doubles as ints
    assert_eq!(at(&v, "f"), &Value::Int(150));
    assert_eq!(at(&v, "g"), &Value::Float(1.25));
    assert_eq!(at(&v, "t"), &Value::Bool(true));
    assert_eq!(at(&v, "n"), &Value::Null);
    assert_eq!(at(&v, "ver"), &s("1.0.0"));
    assert_eq!(at(&v, "ip"), &s("192.168.0.1"));
    assert_eq!(at(&v, "mix"), &s("10abc"));
    assert_eq!(at(&v, "lead"), &Value::Int(7));
}

#[test]
fn arrays() {
    let v = parse("a: [1, 2, 3,]\nb: [\n  1\n  \"x\"\n\n  [2]\n]\nc: []\nd: [1] [2]");
    assert_eq!(at(&v, "a"), &Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
    assert_eq!(at(&v, "b"), &Value::List(vec![Value::Int(1), s("x"), Value::List(vec![Value::Int(2)])]));
    assert_eq!(at(&v, "c"), &Value::List(vec![]));
    assert_eq!(at(&v, "d"), &Value::List(vec![Value::Int(1), Value::Int(2)]));
}

#[test]
fn object_merging_and_overrides() {
    let v = parse("o { x: 1 }\no { y: 2 }\no.z: 3\nr: 1\nr: { k: 1 }\nq: { k: 1 }\nq: 2\nm: {a: 1} {b: 2}");
    assert_eq!(at(&v, "o.x"), &Value::Int(1));
    assert_eq!(at(&v, "o.y"), &Value::Int(2));
    assert_eq!(at(&v, "o.z"), &Value::Int(3));
    assert_eq!(at(&v, "r.k"), &Value::Int(1));
    assert_eq!(at(&v, "q"), &Value::Int(2));
    assert_eq!((at(&v, "m.a"), at(&v, "m.b")), (&Value::Int(1), &Value::Int(2)));
}

#[test]
fn concatenation() {
    let v = parse("a: \"x\" y \"z\"\nb: 1  2.50\nc: foo ${?missing}");
    assert_eq!(at(&v, "a"), &s("x y z"));
    assert_eq!(at(&v, "b"), &s("1  2.50"));
    assert_eq!(at(&v, "c"), &s("foo "));
}

#[test]
fn substitutions() {
    let v = parse(
        "base { x: 1 }\np: ${base.x}\nq: pre-${base.x}\nobj: ${base} { y: 2 }\nlater: ${def}\ndef: 7\nopt: 5\nopt: ${?nope}\nl: [1]\nl: ${l} [2]\nadd += 3\nadd += 4\ngone: ${?nope}",
    );
    assert_eq!(at(&v, "p"), &Value::Int(1));
    assert_eq!(at(&v, "q"), &s("pre-1"));
    assert_eq!((at(&v, "obj.x"), at(&v, "obj.y")), (&Value::Int(1), &Value::Int(2)));
    assert_eq!(at(&v, "later"), &Value::Int(7));
    assert_eq!(at(&v, "opt"), &Value::Int(5));
    assert_eq!(at(&v, "l"), &Value::List(vec![Value::Int(1), Value::Int(2)]));
    assert_eq!(at(&v, "add"), &Value::List(vec![Value::Int(3), Value::Int(4)]));
    assert!(v.get("gone").is_none());
}

#[test]
fn environment_fallback() {
    let (key, value) =
        std::env::vars().find(|(k, _)| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).unwrap();
    let v = parse(&format!("e: ${{{key}}}"));
    assert_eq!(at(&v, "e"), &s(&value));
}

#[test]
fn includes_like_bluemap() {
    let dir = tempfile::tempdir().unwrap();
    let inc = dir.path().join("inc.conf");
    std::fs::write(&inc, "t: included\nn { k: ${a} }").unwrap();
    let file = inc.display().to_string().replace('\\', "/");
    let src = format!(
        "include \"inc.conf\"\ninclude classpath(\"inc.conf\")\ninclude file(\"{file}\")\nx {{ include file(\"{file}\") }}\na: 1"
    );
    let v = parse(&src);
    assert_eq!(at(&v, "t"), &s("included"));
    assert_eq!(at(&v, "x.t"), &s("included"));
    assert_eq!(at(&v, "x.n.k"), &Value::Int(1), "relative lookup falls back to the root");
    assert!(parse("include file(\"no/such/file.conf\")\na: 1").get("a").is_some());
    assert!(err("include required(file(\"no/such/file.conf\"))").message.contains("not found"));
    assert!(err("include = 5").message.contains("quoted file name"));
}

#[test]
fn json_documents() {
    let v = parse("{ \"a\": [1, 2.5, true, null], \"b\": { \"c\": \"d\\/e\" } }");
    assert_eq!(at(&v, "a"), &Value::List(vec![Value::Int(1), Value::Float(2.5), Value::Bool(true), Value::Null]));
    assert_eq!(at(&v, "b.c"), &s("d/e"));
}

#[test]
fn errors_point_at_the_problem() {
    let cases: &[(&str, usize, &str)] = &[
        ("a: 1\nb: {\n  c: 2\n", 4, "missing '}'"),
        ("a: [1,\n2\n", 3, "missing ']'"),
        ("a: 1\n}\n", 2, "unbalanced '}'"),
        ("a: 1\nurl: http://x\n", 2, "enclose the value in double quotes"),
        ("a: 1\nb: \"open\nc: 2\n", 2, "unterminated quoted string"),
        ("a: 1\nb: x@y\n", 2, "reserved character '@'"),
        ("a: 1\nb: ${nope}\n", 2, "could not resolve substitution ${nope}"),
        ("a: [1] {x: 1}\n", 1, "cannot concatenate a list with an object"),
        ("a: ${b}\nb: ${a}\n", 1, "cycle"),
        ("a\n", 1, "has no value"),
        ("a: [1,,2]", 1, "two commas"),
        ("a: \"\\q\"", 1, "invalid escape"),
        ("a..b: 1", 1, "empty path element"),
        ("[1, 2]", 1, "must be an object"),
    ];
    for (src, line, needle) in cases {
        let e = err(src);
        assert_eq!(e.line, *line, "{src:?} → {e}");
        assert!(e.message.contains(needle), "{src:?} → {e}");
        assert!(e.to_string().starts_with(&format!("t.conf:{line}:")), "{e}");
    }
}

#[test]
fn file_errors_carry_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.conf");
    std::fs::write(&path, "accept-download: true\nrender-thread-count: [\n").unwrap();
    let e = hocon::parse_file(&path).unwrap_err().to_string();
    assert!(e.contains("core.conf:3:1"), "{e}");
    std::fs::write(&path, [0xff, 0xfe]).unwrap();
    assert!(hocon::parse_file(&path).unwrap_err().to_string().contains("UTF-8"));
    assert!(hocon::parse_file(&dir.path().join("missing.conf")).unwrap_err().to_string().contains("missing.conf"));
}
