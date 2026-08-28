//! Parsing PL/pgSQL function bodies into sqlparser statements.

use alloc::string::ToString;

use sqlparser::{
    ast::{
        BeginEndStatements, ReturnStatement, ReturnStatementValue, Statement,
        helpers::attached_token::AttachedToken,
    },
    dialect::PostgreSqlDialect,
    keywords::Keyword,
    parser::Parser,
    tokenizer::{Token, TokenWithSpan, Tokenizer, Word},
};

use crate::{Error, PlPgSqlContext, PlPgSqlPreprocessor, Scanner};

/// Preprocesses, tokenizes, and parses a PL/pgSQL function body.
///
/// A trailing `RETURN NEW` or `RETURN OLD` statement is removed.
///
/// # Errors
///
/// Returns an [`Error`] when the body is malformed or cannot be parsed.
pub fn parse_body(
    name: &str,
    raw_body: &str,
) -> Result<(BeginEndStatements, PlPgSqlContext), Error> {
    let body = raw_body.trim().trim_end_matches(';').trim();

    if PlPgSqlPreprocessor::has_exception_handler(body) {
        return Err(Error::ExceptionHandler {
            name: name.to_string(),
        });
    }

    if Scanner::new(body).has_unterminated_dollar_quote() {
        return Err(Error::UnterminatedDollarQuote {
            name: name.to_string(),
        });
    }

    let (preprocessed_body, context) = PlPgSqlPreprocessor::preprocess(body)?;
    let dialect = PostgreSqlDialect {};
    let tokens = Tokenizer::new(&dialect, &preprocessed_body)
        .tokenize()
        .map_err(|source| Error::Tokenization {
            name: name.to_string(),
            body: preprocessed_body.clone(),
            source,
        })?;

    let begin_idx = tokens
        .iter()
        .position(|token| matches!(token, Token::Word(word) if word.keyword == Keyword::BEGIN))
        .ok_or_else(|| Error::MissingBeginBlock {
            name: name.to_string(),
            body: preprocessed_body.clone(),
        })?;
    // The END must close the BEGIN, or a stray earlier one slices backwards.
    let end_idx = tokens
        .iter()
        .rposition(|token| matches!(token, Token::Word(word) if word.keyword == Keyword::END))
        // MUTANT: equivalent: `>=` cannot differ, since one token is either
        // BEGIN or END and never both, so the two indexes never coincide.
        .filter(|&end| end > begin_idx)
        .ok_or_else(|| Error::MissingEndBlock {
            name: name.to_string(),
            body: preprocessed_body.clone(),
        })?;

    let body_tokens = tokens[begin_idx + 1..end_idx].to_vec();
    let mut statements = Parser::new(&dialect)
        .with_tokens(body_tokens)
        .parse_statements()
        .map_err(|source| Error::ParseStatements {
            name: name.to_string(),
            body: preprocessed_body.clone(),
            source,
        })?;

    if let Some(Statement::Return(ReturnStatement {
        value: Some(ReturnStatementValue::Expr(expr)),
    })) = statements.last()
    {
        let string_expr = expr.to_string();
        if string_expr == "NEW" || string_expr == "OLD" {
            statements.pop();
        }
    }

    Ok((
        BeginEndStatements {
            begin_token: AttachedToken(TokenWithSpan::wrap(Token::Word(Word {
                value: "BEGIN".into(),
                quote_style: None,
                keyword: Keyword::BEGIN,
            }))),
            statements,
            end_token: AttachedToken(TokenWithSpan::wrap(Token::Word(Word {
                value: "END".into(),
                quote_style: None,
                keyword: Keyword::END,
            }))),
        },
        context,
    ))
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::parse_body;
    use crate::Error;
    use sqlparser::ast::Statement;

    #[test]
    fn trailing_return_new_is_stripped() {
        let body = "BEGIN\n  INSERT INTO t VALUES (1);\n  RETURN NEW;\nEND;";
        let (parsed, _) = parse_body("f", body).unwrap();
        assert!(
            parsed
                .statements
                .iter()
                .all(|s| !matches!(s, Statement::Return(_)))
        );
    }

    #[test]
    fn trailing_return_value_is_kept() {
        let body = "BEGIN\n  RETURN 1;\nEND;";
        let (parsed, _) = parse_body("f", body).unwrap();
        assert!(matches!(
            parsed.statements.last(),
            Some(Statement::Return(_))
        ));
    }

    #[test]
    fn missing_begin_block_is_refused() {
        assert!(matches!(
            parse_body("f", "SELECT 1;").unwrap_err(),
            Error::MissingBeginBlock { .. }
        ));
    }

    #[test]
    fn parse_error_is_reported() {
        assert!(matches!(
            parse_body("f", "BEGIN\n  SELECT FROM;\nEND;").unwrap_err(),
            Error::ParseStatements { .. }
        ));
    }

    #[test]
    fn unterminated_dollar_quote_is_refused() {
        assert!(matches!(
            parse_body("f", "BEGIN\n  x := $tag$abc;\nEND;").unwrap_err(),
            Error::UnterminatedDollarQuote { .. }
        ));
    }

    #[test]
    fn an_end_before_the_begin_is_refused() {
        assert!(matches!(
            parse_body("f", "END; BEGIN SELECT 1;").unwrap_err(),
            Error::MissingEndBlock { .. }
        ));
    }
}
