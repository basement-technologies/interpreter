use dougterpreter::{
    Interpreter,
    parsing::Parser,
    values::tape::{LiteralHeader, Mutator, MutatorView, StickyImmixHeap},
};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::env;
    let f: String = env::args().nth(1).expect("Expected input douglang file");

    let heap: StickyImmixHeap<LiteralHeader> = StickyImmixHeap::new();
    let scope = MutatorView::new_with(&heap);
    let parser = Parser::new();
    let mut i = Interpreter::new(parser);
    i.run(&scope, f)?;
    Ok(())
}
