use unicode_width::UnicodeWidthStr;

pub fn display_columns(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub fn display_columns_until_char(text: &str, char_count: usize) -> usize {
    match text.char_indices().nth(char_count) {
        Some((byte_index, _)) => display_columns(&text[..byte_index]),
        None => display_columns(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_columns_count_cjk_as_two_and_combining_as_zero() {
        assert_eq!(display_columns("a"), 1);
        assert_eq!(display_columns("中"), 2);
        assert_eq!(display_columns("e\u{0301}"), 1);
        assert_eq!(display_columns("a中e\u{0301}"), 4);
    }

    #[test]
    fn display_columns_until_char_uses_character_cursor_positions() {
        assert_eq!(display_columns_until_char("a中e\u{0301}", 0), 0);
        assert_eq!(display_columns_until_char("a中e\u{0301}", 1), 1);
        assert_eq!(display_columns_until_char("a中e\u{0301}", 2), 3);
        assert_eq!(display_columns_until_char("a中e\u{0301}", 3), 4);
        assert_eq!(display_columns_until_char("a中e\u{0301}", 4), 4);
    }
}
