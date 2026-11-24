//! Veyra's HTML parser: tokenizer plus tree construction into `veyra-dom`.

mod parser;
mod tokenizer;

pub use parser::{is_block_element, parse_document, parse_fragment, ParserStats};
pub use tokenizer::{decode_entities, Token, Tokenizer};
