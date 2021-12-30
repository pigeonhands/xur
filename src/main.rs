mod ast;
mod lexer;
mod xurvm;

use crate::ast::parser::Parser;
use crate::lexer::{Token, TokenKind};
use lexer::Tokenizer;
use xurvm::XurVM;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let parser = {
        let t: Vec<Token> = Tokenizer::new(
            r#"
            "to_int" % ("ff", 16)
        "#,
        )
        .filter(|t| !matches!(t.kind, TokenKind::Whitespace))
        .collect();

        for token in &t {
            println!("{:?}", token);
        }
        Parser::new(t)
    };

    let mut vm = xurvm::XurVM::new();

    for statement in parser {
        let statement = statement?;
        println!("stmnt {:#?}", &statement);
        let res = vm.execute(statement)?;
        println!("{:#?}", &res);
    }

    Ok(())
}
