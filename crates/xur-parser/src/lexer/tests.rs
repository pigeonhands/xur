use super::*;
use crate::token::{Keyword, TokenKind};
use xur_span::Span;

/// Lexes `src`, asserting it contains no errors, and returns the token kinds.
fn lex(src: &str) -> Vec<TokenKind<'_>> {
    let (tokens, errors) = Lexer::new(src).tokenise();
    assert_eq!(errors, [], "unexpected lex errors in {src:?}");
    tokens.into_iter().map(|t| t.kind).collect()
}

/// Lexes `src`, returning the token kinds and the errors.
fn lex_with_errors(src: &str) -> (Vec<TokenKind<'_>>, Vec<LexError>) {
    let (tokens, errors) = Lexer::new(src).tokenise();
    (tokens.into_iter().map(|t| t.kind).collect(), errors)
}

/// Lexes `src` and returns only its errors.
fn lex_errors(src: &str) -> Vec<LexError> {
    lex_with_errors(src).1
}

fn error(kind: LexErrorKind, start: usize, end: usize) -> LexError {
    LexError {
        kind,
        span: Span::new(start, end),
    }
}

fn kw<'a>(k: Keyword) -> TokenKind<'a> {
    TokenKind::Keyword(k)
}
fn id<'a>(s: &'a str) -> TokenKind<'a> {
    TokenKind::Ident(s)
}

#[test]
fn test_use_statement() {
    let kinds = lex("use std;");
    assert_eq!(
        kinds,
        vec![kw(Keyword::Use), id("std"), TokenKind::Semi, TokenKind::Eof]
    );
}

#[test]
fn test_path_access() {
    let kinds = lex("std::io");
    assert_eq!(
        kinds,
        vec![id("std"), TokenKind::ColonColon, id("io"), TokenKind::Eof]
    );
}

#[test]
fn test_dot_access() {
    let kinds = lex("math.add");
    assert_eq!(
        kinds,
        vec![id("math"), TokenKind::Dot, id("add"), TokenKind::Eof]
    );
}

#[test]
fn test_fn_signature() {
    let kinds = lex("pub fn add(a: u32, b: u32) -> u32");
    assert_eq!(
        kinds,
        vec![
            kw(Keyword::Pub),
            kw(Keyword::Fn),
            id("add"),
            TokenKind::LParen,
            id("a"),
            TokenKind::Colon,
            id("u32"),
            TokenKind::Comma,
            id("b"),
            TokenKind::Colon,
            id("u32"),
            TokenKind::RParen,
            TokenKind::Arrow,
            id("u32"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_string_literal() {
    let kinds = lex(r#""Hello, World""#);
    assert_eq!(
        kinds,
        vec![TokenKind::StringLit("Hello, World"), TokenKind::Eof]
    );
}

#[test]
fn test_string_interpolation_holes_pass_through() {
    let kinds = lex(r#""1 + 2 = {}""#);
    assert_eq!(
        kinds,
        vec![TokenKind::StringLit("1 + 2 = {}"), TokenKind::Eof]
    );
}

#[test]
fn test_number_literals() {
    let kinds = lex("42 2.5 1_000_000");
    assert_eq!(
        kinds,
        vec![
            TokenKind::IntLit("42"),
            TokenKind::FloatLit("2.5"),
            TokenKind::IntLit("1_000_000"),
            TokenKind::Eof
        ]
    );
}

#[test]
fn test_line_comment_skipped() {
    let kinds = lex("let x // this is a comment\nlet y");
    assert_eq!(
        kinds,
        vec![
            kw(Keyword::Let),
            id("x"),
            kw(Keyword::Let),
            id("y"),
            TokenKind::Eof
        ]
    );
}

#[test]
fn test_nested_block_comments() {
    let kinds = lex("let a; /* outer /* inner */ still outer */ let b;");
    assert_eq!(
        kinds,
        vec![
            kw(Keyword::Let),
            id("a"),
            TokenKind::Semi,
            kw(Keyword::Let),
            id("b"),
            TokenKind::Semi,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_unicode_identifiers() {
    let kinds = lex("let α_beta = 10;");
    assert_eq!(
        kinds,
        vec![
            kw(Keyword::Let),
            id("α_beta"),
            TokenKind::Eq,
            TokenKind::IntLit("10"),
            TokenKind::Semi,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_const_import() {
    let kinds = lex(r#"const math = std::import("./math.xur");"#);
    assert_eq!(
        kinds,
        vec![
            kw(Keyword::Const),
            id("math"),
            TokenKind::Eq,
            id("std"),
            TokenKind::ColonColon,
            id("import"),
            TokenKind::LParen,
            TokenKind::StringLit("./math.xur"),
            TokenKind::RParen,
            TokenKind::Semi,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_loop_constructs() {
    let kinds = lex("for i in 0..10 { if i == 5 { break; } else { continue; } }");
    assert_eq!(
        kinds,
        vec![
            kw(Keyword::For),
            id("i"),
            kw(Keyword::In),
            TokenKind::IntLit("0"),
            TokenKind::DotDot,
            TokenKind::IntLit("10"),
            TokenKind::LBrace,
            kw(Keyword::If),
            id("i"),
            TokenKind::EqEq,
            TokenKind::IntLit("5"),
            TokenKind::LBrace,
            kw(Keyword::Break),
            TokenKind::Semi,
            TokenKind::RBrace,
            kw(Keyword::Else),
            TokenKind::LBrace,
            kw(Keyword::Continue),
            TokenKind::Semi,
            TokenKind::RBrace,
            TokenKind::RBrace,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_range_operators() {
    let kinds = lex("0..10 1..=5");
    assert_eq!(
        kinds,
        vec![
            TokenKind::IntLit("0"),
            TokenKind::DotDot,
            TokenKind::IntLit("10"),
            TokenKind::IntLit("1"),
            TokenKind::DotDotEq,
            TokenKind::IntLit("5"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_turbofish() {
    let kinds = lex("print_name::<ZeroPacked>();");
    assert_eq!(
        kinds,
        vec![
            id("print_name"),
            TokenKind::ColonColon,
            TokenKind::Lt,
            id("ZeroPacked"),
            TokenKind::Gt,
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::Semi,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_attributes_and_struct() {
    let kinds = lex("#[repr(C)] struct Foo { a: u32 }");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Hash,
            TokenKind::LBracket,
            id("repr"),
            TokenKind::LParen,
            id("C"),
            TokenKind::RParen,
            TokenKind::RBracket,
            kw(Keyword::Struct),
            id("Foo"),
            TokenKind::LBrace,
            id("a"),
            TokenKind::Colon,
            id("u32"),
            TokenKind::RBrace,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_keyword_display() {
    assert_eq!(TokenKind::Keyword(Keyword::Fn).to_string(), "fn");
    assert_eq!(TokenKind::Keyword(Keyword::Use).to_string(), "use");
    assert_eq!(TokenKind::Keyword(Keyword::Struct).to_string(), "struct");
    assert_eq!(TokenKind::Ident("foo").to_string(), "<ident>");
    assert_eq!(TokenKind::ColonColon.to_string(), "::");
    assert_eq!(TokenKind::FatArrow.to_string(), "=>");
    assert_eq!(TokenKind::Pipe.to_string(), "|");
    assert_eq!(TokenKind::Underscore.to_string(), "_");
    assert_eq!(TokenKind::Hash.to_string(), "#");
    assert_eq!(TokenKind::Arrow.to_string(), "->");
    assert_eq!(TokenKind::LBrace.to_string(), "{");
    assert_eq!(TokenKind::RBrace.to_string(), "}");
}

#[test]
fn test_examples_lex_cleanly() {
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../example"));
    let mut count = 0;
    let mut dirs = vec![dir.to_owned()];
    while let Some(d) = dirs.pop() {
        for entry in std::fs::read_dir(&d).expect("read example dir") {
            let path = entry.expect("read example entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "xur") {
                let src = std::fs::read_to_string(&path).expect("read example file");
                assert_eq!(Lexer::new(&src).tokenise().1, [], "lex errors in {path:?}");
                count += 1;
            }
        }
    }
    assert!(count > 0, "no examples found in {}", dir.display());
}

#[test]
fn test_full_main() {
    let src = r#"
        use std;
        const math = std::import("./math.xur");
        fn main() {
            let io = std::io;
            io::println("Hello, World");
            io::println("1 + 2 = {}", math.add(1, 2));
        }
    "#;
    let kinds = lex(src);
    assert_eq!(kinds.last(), Some(&TokenKind::Eof));
}

#[test]
fn test_spans_are_byte_ranges() {
    let src = "let α = \"hi\";";
    let (tokens, _) = Lexer::new(src).tokenise();
    let slices: Vec<_> = tokens
        .iter()
        .map(|t| &src[t.span.start..t.span.end])
        .collect();
    assert_eq!(slices, ["let", "α", "=", "\"hi\"", ";", ""]);
    assert_eq!(tokens.last().unwrap().span, Span::new(src.len(), src.len()));
}

#[test]
fn test_unexpected_char_is_skipped() {
    let (kinds, errors) = lex_with_errors("let $x");
    assert_eq!(kinds, [kw(Keyword::Let), id("x"), TokenKind::Eof]);
    assert_eq!(errors, [error(LexErrorKind::UnexpectedChar('$'), 4, 5)]);
}

#[test]
fn test_unterminated_string_stops_at_newline() {
    let (kinds, errors) = lex_with_errors("\"oops\nlet x;");
    assert_eq!(
        kinds,
        [
            TokenKind::StringLit("oops"),
            kw(Keyword::Let),
            id("x"),
            TokenKind::Semi,
            TokenKind::Eof
        ]
    );
    assert_eq!(errors, [error(LexErrorKind::UnterminatedString, 0, 5)]);
}

#[test]
fn test_unterminated_block_comment() {
    assert_eq!(
        lex_errors("a /* outer /* inner */"),
        [error(LexErrorKind::UnterminatedBlockComment, 2, 22)]
    );
}

#[test]
fn test_number_literals_keep_their_text() {
    // Value conversion, and so overflow checking, happens in the parser.
    assert_eq!(
        lex("18_446_744_073_709_551_616 1_0.0_1"),
        [
            TokenKind::IntLit("18_446_744_073_709_551_616"),
            TokenKind::FloatLit("1_0.0_1"),
            TokenKind::Eof
        ]
    );
}

#[test]
fn test_int_followed_by_method_call() {
    assert_eq!(
        lex("1.max"),
        [
            TokenKind::IntLit("1"),
            TokenKind::Dot,
            id("max"),
            TokenKind::Eof
        ]
    );
}

#[test]
fn test_all_errors_are_reported() {
    let kinds: Vec<_> = lex_errors("$ \"a\n~").into_iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            LexErrorKind::UnexpectedChar('$'),
            LexErrorKind::UnterminatedString,
            LexErrorKind::UnexpectedChar('~'),
        ]
    );
}

#[test]
fn test_closure() {
    assert_eq!(
        lex("|x: u32| x + 1"),
        [
            TokenKind::Pipe,
            id("x"),
            TokenKind::Colon,
            id("u32"),
            TokenKind::Pipe,
            id("x"),
            TokenKind::Plus,
            TokenKind::IntLit("1"),
            TokenKind::Eof,
        ]
    );
    assert_eq!(
        lex("|| a || b"),
        [
            TokenKind::PipePipe,
            id("a"),
            TokenKind::PipePipe,
            id("b"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_match_arms() {
    assert_eq!(
        lex("match r { Ok(v) => v, _ => 0 }"),
        [
            kw(Keyword::Match),
            id("r"),
            TokenKind::LBrace,
            id("Ok"),
            TokenKind::LParen,
            id("v"),
            TokenKind::RParen,
            TokenKind::FatArrow,
            id("v"),
            TokenKind::Comma,
            TokenKind::Underscore,
            TokenKind::FatArrow,
            TokenKind::IntLit("0"),
            TokenKind::RBrace,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_underscore_prefixed_ident() {
    assert_eq!(
        lex("_ _x __"),
        [TokenKind::Underscore, id("_x"), id("__"), TokenKind::Eof]
    );
}

#[test]
fn test_try_operator() {
    assert_eq!(
        lex("parse(s)?"),
        [
            id("parse"),
            TokenKind::LParen,
            id("s"),
            TokenKind::RParen,
            TokenKind::Question,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_enum_declaration() {
    assert_eq!(
        lex("pub enum Result<T, E> { Ok(T) Err(E) }"),
        [
            kw(Keyword::Pub),
            kw(Keyword::Enum),
            id("Result"),
            TokenKind::Lt,
            id("T"),
            TokenKind::Comma,
            id("E"),
            TokenKind::Gt,
            TokenKind::LBrace,
            id("Ok"),
            TokenKind::LParen,
            id("T"),
            TokenKind::RParen,
            id("Err"),
            TokenKind::LParen,
            id("E"),
            TokenKind::RParen,
            TokenKind::RBrace,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_impl_and_self() {
    assert_eq!(
        lex("impl S { fn f(self) {} }"),
        [
            kw(Keyword::Impl),
            id("S"),
            TokenKind::LBrace,
            kw(Keyword::Fn),
            id("f"),
            TokenKind::LParen,
            id("self"),
            TokenKind::RParen,
            TokenKind::LBrace,
            TokenKind::RBrace,
            TokenKind::RBrace,
            TokenKind::Eof,
        ]
    );
}
