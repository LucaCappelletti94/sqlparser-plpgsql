#![no_main]

use libfuzzer_sys::fuzz_target;
use sqlparser_plpgsql::PlPgSqlPreprocessor;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if text.len() > 2048 {
            return;
        }
        let _ = PlPgSqlPreprocessor::preprocess(text);
        let _ = PlPgSqlPreprocessor::has_exception_handler(text);
    }
});
