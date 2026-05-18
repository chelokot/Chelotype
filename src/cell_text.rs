use crate::terminal_grid::TerminalCell;

pub fn push_cell_text(out: &mut String, cell: &TerminalCell) {
    if is_wide_spacer(cell) {
        return;
    }
    out.push_str(&cell.text);
}

pub fn cells_to_text(cells: &[TerminalCell]) -> String {
    let mut out = String::new();
    for cell in cells {
        push_cell_text(&mut out, cell);
    }
    out
}

pub fn lines_to_text(lines: &[Vec<TerminalCell>]) -> String {
    let mut out = String::new();
    for (idx, line) in lines.iter().enumerate() {
        out.push_str(&cells_to_text(line));
        if idx + 1 != lines.len() {
            out.push('\n');
        }
    }
    out
}

pub fn is_wide_spacer(cell: &TerminalCell) -> bool {
    cell.wide_spacer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(text: &str) -> TerminalCell {
        TerminalCell {
            text: text.to_string(),
            ..TerminalCell::blank()
        }
    }

    #[test]
    fn text_preserves_zerowidth_combining_marks() {
        let composed = cell("e\u{0301}");
        assert_eq!(
            cells_to_text(&[cell("a"), composed, cell("b")]),
            "ae\u{0301}b"
        );
    }

    #[test]
    fn text_skips_wide_spacer_cells() {
        let mut wide = cell("中");
        wide.wide = true;
        let mut spacer = TerminalCell::blank();
        spacer.wide_spacer = true;
        assert_eq!(cells_to_text(&[cell("a"), wide, spacer, cell("b")]), "a中b");
    }
}
