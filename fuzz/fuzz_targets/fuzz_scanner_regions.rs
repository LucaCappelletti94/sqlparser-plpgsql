#![no_main]

use libfuzzer_sys::fuzz_target;
use sqlparser_plpgsql::Scanner;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if text.len() > 4096 {
            return;
        }
        let scanner = Scanner::new(text);
        let regions = scanner.regions();
        // P1 invariant: a regression to index += 1 triggers this immediately.
        assert_eq!(regions.len(), text.len());
        let _ = scanner.requote_dollar_literals();
    }
});
