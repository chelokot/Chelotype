use crate::selection::{GridPoint, SelectionRange, line_significant_len};
use crate::terminal_grid::{TerminalContent, TerminalSemanticPrompt};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CommandBlock {
    pub prompt_start_row: usize,
    pub prompt_end_row: usize,
    pub end_row: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandBlockDirection {
    Previous,
    Next,
}

impl CommandBlock {
    pub fn output_start_row(&self) -> Option<usize> {
        (self.prompt_end_row < self.end_row).then_some(self.prompt_end_row + 1)
    }

    pub fn output_end_row(&self) -> Option<usize> {
        self.output_start_row().map(|_| self.end_row)
    }
}

pub fn command_block_output_range(
    content: &TerminalContent,
    block: &CommandBlock,
) -> Option<SelectionRange> {
    let output_start = block.output_start_row()?;
    let output_end = (output_start..=block.end_row).rev().find(|row| {
        content
            .lines
            .get(*row)
            .is_some_and(|line| line_significant_len(line) > 0)
    })?;
    let end_column = line_significant_len(content.lines.get(output_end)?);
    (end_column > 0).then_some(SelectionRange::new(
        GridPoint {
            row: output_start,
            column: 0,
        },
        GridPoint {
            row: output_end,
            column: end_column,
        },
    ))
}

pub fn command_block_output_range_near_cursor(
    content: &TerminalContent,
    direction: CommandBlockDirection,
) -> Option<SelectionRange> {
    let cursor_row = usize::try_from(content.cursor_line).ok()?;
    let blocks = command_blocks(content);
    match direction {
        CommandBlockDirection::Previous => blocks
            .iter()
            .rev()
            .filter(|block| block.prompt_start_row < cursor_row)
            .find_map(|block| command_block_output_range(content, block)),
        CommandBlockDirection::Next => blocks
            .iter()
            .filter(|block| block.prompt_start_row >= cursor_row)
            .find_map(|block| command_block_output_range(content, block)),
    }
}

pub fn command_blocks(content: &TerminalContent) -> Vec<CommandBlock> {
    let mut blocks = Vec::new();
    let mut current: Option<CommandBlock> = None;
    for row in 0..content.lines.len() {
        let semantic_prompt = content
            .line_metadata
            .get(row)
            .map(|metadata| metadata.semantic_prompt)
            .unwrap_or_default();
        match semantic_prompt {
            TerminalSemanticPrompt::Prompt => {
                if let Some(mut block) = current.take() {
                    block.end_row = row.saturating_sub(1);
                    blocks.push(block);
                }
                current = Some(CommandBlock {
                    prompt_start_row: row,
                    prompt_end_row: row,
                    end_row: row,
                });
            }
            TerminalSemanticPrompt::Continuation => {
                if let Some(block) = current.as_mut() {
                    if block.prompt_end_row + 1 == row {
                        block.prompt_end_row = row;
                    }
                    block.end_row = row;
                }
            }
            TerminalSemanticPrompt::None => {
                if let Some(block) = current.as_mut() {
                    block.end_row = row;
                }
            }
        }
    }
    if let Some(block) = current {
        blocks.push(block);
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_grid::{MouseMode, TerminalCell, TerminalColors, TerminalLineMetadata};

    fn content(metadata: Vec<TerminalSemanticPrompt>) -> TerminalContent {
        TerminalContent {
            lines: metadata
                .iter()
                .map(|_| vec![TerminalCell::blank()].into())
                .collect(),
            line_metadata: metadata
                .into_iter()
                .map(|semantic_prompt| TerminalLineMetadata {
                    semantic_prompt,
                    ..TerminalLineMetadata::default()
                })
                .collect(),
            cursor_line: 0,
            cursor_col: 0,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        }
    }

    #[test]
    fn builds_blocks_from_semantic_prompt_rows() {
        let blocks = command_blocks(&content(vec![
            TerminalSemanticPrompt::None,
            TerminalSemanticPrompt::Prompt,
            TerminalSemanticPrompt::Continuation,
            TerminalSemanticPrompt::None,
            TerminalSemanticPrompt::None,
            TerminalSemanticPrompt::Prompt,
            TerminalSemanticPrompt::None,
        ]));
        assert_eq!(
            blocks,
            vec![
                CommandBlock {
                    prompt_start_row: 1,
                    prompt_end_row: 2,
                    end_row: 4,
                },
                CommandBlock {
                    prompt_start_row: 5,
                    prompt_end_row: 5,
                    end_row: 6,
                },
            ]
        );
        assert_eq!(blocks[0].output_start_row(), Some(3));
        assert_eq!(blocks[0].output_end_row(), Some(4));
    }

    #[test]
    fn does_not_guess_blocks_without_semantic_prompt() {
        assert!(command_blocks(&content(vec![TerminalSemanticPrompt::None])).is_empty());
    }

    #[test]
    fn output_range_trims_blank_tail_and_keeps_full_output_body() {
        let mut content = content(vec![
            TerminalSemanticPrompt::Prompt,
            TerminalSemanticPrompt::None,
            TerminalSemanticPrompt::None,
            TerminalSemanticPrompt::None,
            TerminalSemanticPrompt::Prompt,
        ]);
        content.lines = vec![
            vec![TerminalCell::blank()].into(),
            "OUT_1"
                .chars()
                .map(|ch| TerminalCell {
                    text: ch.to_string().into(),
                    ..TerminalCell::blank()
                })
                .collect::<Vec<_>>()
                .into(),
            "OUT_2"
                .chars()
                .map(|ch| TerminalCell {
                    text: ch.to_string().into(),
                    ..TerminalCell::blank()
                })
                .collect::<Vec<_>>()
                .into(),
            vec![TerminalCell::blank()].into(),
            vec![TerminalCell::blank()].into(),
        ];
        let blocks = command_blocks(&content);

        assert_eq!(
            command_block_output_range(&content, &blocks[0]),
            Some(SelectionRange::new(
                GridPoint { row: 1, column: 0 },
                GridPoint { row: 2, column: 5 },
            ))
        );
    }

    #[test]
    fn output_range_near_cursor_selects_previous_or_next_block_output() {
        let mut content = content(vec![
            TerminalSemanticPrompt::Prompt,
            TerminalSemanticPrompt::None,
            TerminalSemanticPrompt::Prompt,
            TerminalSemanticPrompt::None,
            TerminalSemanticPrompt::Prompt,
        ]);
        content.lines = vec![
            vec![TerminalCell::blank()].into(),
            "PREV"
                .chars()
                .map(|ch| TerminalCell {
                    text: ch.to_string().into(),
                    ..TerminalCell::blank()
                })
                .collect::<Vec<_>>()
                .into(),
            vec![TerminalCell::blank()].into(),
            "NEXT"
                .chars()
                .map(|ch| TerminalCell {
                    text: ch.to_string().into(),
                    ..TerminalCell::blank()
                })
                .collect::<Vec<_>>()
                .into(),
            vec![TerminalCell::blank()].into(),
        ];
        content.cursor_line = 2;

        assert_eq!(
            command_block_output_range_near_cursor(&content, CommandBlockDirection::Previous),
            Some(SelectionRange::new(
                GridPoint { row: 1, column: 0 },
                GridPoint { row: 1, column: 4 },
            ))
        );
        assert_eq!(
            command_block_output_range_near_cursor(&content, CommandBlockDirection::Next),
            Some(SelectionRange::new(
                GridPoint { row: 3, column: 0 },
                GridPoint { row: 3, column: 4 },
            ))
        );
    }
}
