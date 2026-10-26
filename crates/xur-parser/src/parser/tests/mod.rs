mod expr;
mod item;
mod lit;
mod pat;
mod show;
mod stmt;
mod ty;

use super::{PResult, Parser};

/// Runs `parse` over the whole of `src`, asserting that it succeeds, consumes
/// all input and reports nothing.
fn parse_ok<'src, T>(src: &'src str, parse: impl FnOnce(&mut Parser<'src>) -> PResult<T>) -> T {
    let mut parser = Parser::new(src);
    let result = parse(&mut parser).and_then(|node| parser.expect_end().map(|()| node));
    let diags = parser.finish();
    match result {
        Ok(node) if diags.is_empty() => node,
        Ok(_) => panic!("errors parsing {src:?}: {diags:#?}"),
        Err(diag) => panic!("failed to parse {src:?}: {diag:#?}\nalso reported: {diags:#?}"),
    }
}

/// Runs `parse` over `src` and returns the messages of every error, whether
/// returned or reported.
fn parse_errors<'src, T>(
    src: &'src str,
    parse: impl FnOnce(&mut Parser<'src>) -> PResult<T>,
) -> Vec<String> {
    let mut parser = Parser::new(src);
    let returned = parse(&mut parser).err();
    parser
        .finish()
        .into_iter()
        .chain(returned)
        .map(|diag| diag.message)
        .collect()
}
