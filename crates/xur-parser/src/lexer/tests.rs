use super::*;
use crate::token::{Keyword, TokenKind};

/// Lexes `src`, asserting it contains no errors, and returns the token kinds.
fn lex(src: &str) -> Vec<TokenKind<'_>> {
    let (tokens, errors) = Lexer::new(src).tokenise();
    assert_eq!(errors, [], "unexpected lex errors in {src:?}");
    tokens.into_iter().map(|t| t.kind).collect()
}

/// Lexes `src` and returns only its errors.
fn lex_errors(src: &str) -> Vec<LexError> {
    Lexer::new(src).tokenise().1
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
            TokenKind::IntLit(42),
            TokenKind::FloatLit(2.5),
            TokenKind::IntLit(1_000_000),
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
            TokenKind::IntLit(10),
            TokenKind::Semi,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn test_iterator_streaming_and_resilient_recovery() {
    let mut lexer = Lexer::new("let 42$ // unknown char $");
    let t1 = lexer.next().unwrap();
    assert_eq!(t1.kind, kw(Keyword::Let));
    let t2 = lexer.next().unwrap();
    assert_eq!(t2.kind, TokenKind::IntLit(42));
    let t3 = lexer.next().unwrap();
    assert_eq!(t3.kind, TokenKind::Error(LexErrorKind::UnexpectedChar('$')));
    let t4 = lexer.next().unwrap();
    assert_eq!(t4.kind, TokenKind::Eof);
    assert!(lexer.next().is_none());
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
            TokenKind::IntLit(0),
            TokenKind::DotDot,
            TokenKind::IntLit(10),
            TokenKind::LBrace,
            kw(Keyword::If),
            id("i"),
            TokenKind::EqEq,
            TokenKind::IntLit(5),
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
            TokenKind::IntLit(0),
            TokenKind::DotDot,
            TokenKind::IntLit(10),
            TokenKind::IntLit(1),
            TokenKind::DotDotEq,
            TokenKind::IntLit(5),
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
            TokenKind::ColonColonLt,
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
    assert_eq!(TokenKind::ColonColonLt.to_string(), "::<");
    assert_eq!(TokenKind::Hash.to_string(), "#");
    assert_eq!(TokenKind::Arrow.to_string(), "->");
    assert_eq!(TokenKind::LBrace.to_string(), "{");
    assert_eq!(TokenKind::RBrace.to_string(), "}");
}

#[test]
fn test_example_main_file() {
    let content = std::fs::read_to_string("../../example/main.xur").expect("read example/main.xur");
    let kinds = lex(&content);
    assert_eq!(kinds.last(), Some(&TokenKind::Eof));
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
    let slices: Vec<_> = tokens.iter().map(|t| t.span.slice(src)).collect();
    assert_eq!(slices, ["let", "α", "=", "\"hi\"", ";", ""]);
    assert_eq!(tokens.last().unwrap().span, Span::new(src.len(), src.len()));
}

#[test]
fn test_unexpected_char() {
    assert_eq!(
        lex_errors("let $x"),
        [error(LexErrorKind::UnexpectedChar('$'), 4, 5)]
    );
}

#[test]
fn test_unterminated_string_stops_at_newline() {
    let src = "\"oops\nlet x;";
    assert_eq!(
        lex_errors(src),
        [error(LexErrorKind::UnterminatedString, 0, 5)]
    );
    let kinds: Vec<_> = Lexer::new(src)
        .tokenise()
        .0
        .into_iter()
        .map(|t| t.kind)
        .collect();
    assert_eq!(
        kinds,
        [kw(Keyword::Let), id("x"), TokenKind::Semi, TokenKind::Eof]
    );
}

#[test]
fn test_unterminated_block_comment() {
    assert_eq!(
        lex_errors("a /* outer /* inner */"),
        [error(LexErrorKind::UnterminatedBlockComment, 2, 22)]
    );
}

#[test]
fn test_int_too_large() {
    assert_eq!(
        lex_errors("18_446_744_073_709_551_616"),
        [error(LexErrorKind::IntTooLarge, 0, 26)]
    );
    assert_eq!(lex("18446744073709551615")[0], TokenKind::IntLit(u64::MAX));
}

#[test]
fn test_all_errors_are_reported() {
    let kinds: Vec<_> = lex_errors("$ \"a\n@").into_iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            LexErrorKind::UnexpectedChar('$'),
            LexErrorKind::UnterminatedString,
            LexErrorKind::UnexpectedChar('@'),
        ]
    );
}
